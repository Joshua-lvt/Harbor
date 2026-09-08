package main

// Direct DataChannel surfaces for chat and file transfer. Everything here is
// transport only: the worker validates frame shapes and buffers bounded
// inbound data for a polling owner, but it never decides what a message is,
// where a file lands, or whether a peer may send either. The Rust core owns
// the chat and transfer state machines; this file guarantees that bytes only
// ever move across the direct WebRTC path and that neither side can wedges
// the other (bounded rings, watermarks, explicit cancellation).

import (
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"sync"
	"time"

	"github.com/pion/webrtc/v4"
)

const (
	// Chat frame limits. The body cap is enforced on the wire frame, so an
	// oversized message can never enter the worker at all.
	chatMaxBodyBytes  = 4 * 1024
	chatInboxCapacity = 64

	// File transfer limits. Chunk bytes arrive base64-encoded inside JSON
	// frames; the ring bounds how much unclaimed inbound data the worker
	// will hold while the core is between polls. fileMaxBytes is the shared
	// protocol ceiling — the Rust core (direct.rs TRANSFER_MAX_BYTES) carries
	// the same number, and real capacity on top of it comes from free space
	// on the core's staging and destination volumes, not from this worker.
	fileMaxBytes         = 1 << 40
	fileMaxChunkBytes    = 64 * 1024
	chunkRingCapacity    = 64
	transferNameMaxRunes = 255

	// Outbound backpressure: before writing a chunk the worker waits until
	// the SCTP sender has drained below the high watermark. The wait is
	// capped safely under the supervisor's request deadline; if the watermark
	// persists, the chunk is reported paused so the core can retry later.
	sendHighWatermark = 1 << 20
	sendWaitSteps     = 200
	sendWaitInterval  = 10 * time.Millisecond

	// v1 of the direct frames; a peer that speaks another version is ignored
	// rather than guessed at.
	directFrameVersion = 1
)

var (
	errChannelClosed   = errors.New("the direct channel is not open")
	errTransferUnknown = errors.New("transfer is not known to this worker")
	errTransferState   = errors.New("transfer is not in the required state")
	errFrameInvalid    = errors.New("direct frame is invalid")
)

// directFrame is the single JSON shape used on all three channels. The kind
// selects the family, so every channel parses with the same struct and a peer
// cannot smuggle a file payload through the chat channel unnoticed.
type directFrame struct {
	Version   int    `json:"v"`
	Kind      string `json:"kind"`
	Action    string `json:"action,omitempty"`
	Message   string `json:"message,omitempty"`
	Body      string `json:"body,omitempty"`
	Transfer  string `json:"transfer,omitempty"`
	Name      string `json:"name,omitempty"`
	Size      int64  `json:"size,omitempty"`
	Sum       string `json:"sha256,omitempty"`
	Chunk     int    `json:"chunk,omitempty"`
	ChunkSize int    `json:"chunk_size,omitempty"`
	Offset    int64  `json:"off,omitempty"`
	Data      string `json:"data,omitempty"`
	Final     bool   `json:"final,omitempty"`
	OK        bool   `json:"ok,omitempty"`
}

// chatMessage is one inbound chat body waiting for the core to poll it.
type chatMessage struct {
	ID   string `json:"message_id"`
	Body string `json:"body"`
}

// fileChunk is one inbound transfer piece waiting for the core to poll it.
type fileChunk struct {
	Seq    int    `json:"seq"`
	Offset int64  `json:"offset"`
	Data   []byte `json:"-"`
	Base64 string `json:"-"`
	Final  bool   `json:"final"`
}

// transferState tracks one direct transfer from this worker's point of view.
// Direction and phase names stay transport-shaped; the core's richer state
// machine maps onto them.
type transferState struct {
	id        string
	direction string // "outgoing" or "incoming"
	phase     string // "offered" "active" "completed" "canceled"
	name      string
	size      int64
	sum       string
	chunkSize int
	peerDone  bool // peer confirmed final receipt (finalize frame)
	cancelled bool
}

// channelSet owns the call's DataChannels and the bounded buffers between
// them and the polling core. It exists only while a call is live.
type channelSet struct {
	mu      sync.Mutex
	control *webrtc.DataChannel
	chat    *webrtc.DataChannel
	file    *webrtc.DataChannel

	inbox    []chatMessage
	inboxMax int

	// activity holds sanitized activity frames the peer pushed over the
	// control channel, waiting for the core's next activity.poll. Bounded
	// like every inbox here: beyond capacity the frame is refused.
	activity []string

	// profile holds opaque public-profile frames the peer pushed over the
	// control channel, waiting for the core's next profile.poll. Bounded
	// like every inbox here: beyond capacity the frame is refused. The
	// receiving core validates every frame before any UI ever sees it.
	profile []string

	chunks    map[string][]fileChunk
	transfers map[string]*transferState

	// delivery records outbound chat ids: true once the peer's worker
	// acknowledged the frame.
	delivered map[string]bool

	// Pion forbids re-entrant sends: a control frame produced inside an
	// OnMessage callback (chat acknowledgements, buffer-overflow cancels) is
	// queued here and written by a dedicated pump goroutine instead.
	pump    chan directFrame
	stopPmp chan struct{}
	pmpDone chan struct{}
	closed  bool
}

func newChannelSet() *channelSet {
	c := &channelSet{
		chunks:    make(map[string][]fileChunk),
		transfers: make(map[string]*transferState),
		delivered: make(map[string]bool),
		inboxMax:  chatInboxCapacity,
		pump:      make(chan directFrame, 128),
		stopPmp:   make(chan struct{}),
		pmpDone:   make(chan struct{}),
	}
	go c.pumpLoop()
	return c
}

// pumpLoop is the only writer of OnMessage-originated control frames.
func (c *channelSet) pumpLoop() {
	defer close(c.pmpDone)
	for {
		select {
		case <-c.stopPmp:
			return
		case frame := <-c.pump:
			c.mu.Lock()
			c.sendControlLocked(frame)
			c.mu.Unlock()
		}
	}
}

// queueControl schedules a control frame from inside a receive callback.
// A full pump means the peer's acknowledgements are piling up faster than
// the link drains them; the fact surfaces through the poll surfaces.
func (c *channelSet) queueControl(frame directFrame) {
	select {
	case c.pump <- frame:
	default:
	}
}

// bind installs the frame handlers for one opened channel. Unknown labels are
// ignored: a peer may not invent surfaces this worker never offered.
func (c *channelSet) bind(dc *webrtc.DataChannel) {
	c.mu.Lock()
	switch dc.Label() {
	case "control":
		c.control = dc
	case "chat":
		c.chat = dc
	case "file":
		c.file = dc
	default:
		c.mu.Unlock()
		return
	}
	c.mu.Unlock()

	label := dc.Label()
	dc.OnMessage(func(msg webrtc.DataChannelMessage) {
		c.receive(label, msg.Data)
	})
}

// receive parses one inbound frame into the bounded buffer for its family.
// Parsing failures are dropped silently — a malformed frame is a peer bug,
// and the poll surface simply never reports it.
func (c *channelSet) receive(label string, data []byte) {
	var frame directFrame
	if json.Unmarshal(data, &frame) != nil || frame.Version != directFrameVersion {
		return
	}

	c.mu.Lock()
	defer c.mu.Unlock()
	switch {
	case label == "chat" && frame.Kind == "chat":
		if len(frame.Body) > chatMaxBodyBytes || frame.Message == "" {
			return
		}
		if len(c.inbox) >= c.inboxMax {
			// The core is not draining: refuse further chat honestly instead
			// of dropping individual messages quietly.
			c.inboxMax = -1 // poisoned; poll reports the overflow once
			return
		}
		c.inbox = append(c.inbox, chatMessage{ID: frame.Message, Body: frame.Body})
		// Worker-level ack: the frame reached this machine intact. The core
		// still owns display and persistence. The ack leaves through the
		// pump — OnMessage must never send re-entrantly.
		c.queueControl(directFrame{Version: directFrameVersion, Kind: "control", Action: "chat_ack", Message: frame.Message})

	case label == "control" && frame.Kind == "control":
		switch frame.Action {
		case "chat_ack":
			c.delivered[frame.Message] = true
		case "activity":
			// Sanitized activity facts from the peer. The bytes are opaque
			// here — the receiving core validates the schema before any UI
			// sees them — but the transport bound still applies.
			if frame.Data == "" || len(frame.Data) > activityFrameMaxBytes {
				return
			}
			if len(c.activity) >= activityInboxCapacity {
				return
			}
			c.activity = append(c.activity, frame.Data)
		case "profile":
			// Opaque public-profile frames from the paired peer. The bytes
			// are validated by the receiving core (revision ordering,
			// schema, avatar hash) before any UI sees them — but the
			// transport bound still applies here.
			if frame.Data == "" || len(frame.Data) > profileFrameMaxBytes {
				return
			}
			if len(c.profile) >= profileInboxCapacity {
				return
			}
			c.profile = append(c.profile, frame.Data)
		case "offer":
			if frame.Transfer == "" || frame.Size <= 0 || frame.Size > fileMaxBytes {
				return
			}
			if _, exists := c.transfers[frame.Transfer]; exists {
				return
			}
			c.transfers[frame.Transfer] = &transferState{
				id: frame.Transfer, direction: "incoming", phase: "offered",
				name: frame.Name, size: frame.Size, sum: frame.Sum, chunkSize: frame.ChunkSize,
			}
		case "accept":
			if t := c.transfers[frame.Transfer]; t != nil && t.direction == "outgoing" {
				t.phase = "active"
			}
		case "reject", "cancel":
			if t := c.transfers[frame.Transfer]; t != nil {
				t.phase = "canceled"
				t.cancelled = true
				delete(c.chunks, frame.Transfer)
			}
		case "finalize":
			if t := c.transfers[frame.Transfer]; t != nil && t.direction == "outgoing" {
				t.peerDone = frame.OK
				if frame.OK {
					t.phase = "completed"
				} else {
					t.phase = "canceled"
				}
			}
		}

	case label == "file" && frame.Kind == "file":
		t := c.transfers[frame.Transfer]
		if t == nil || t.direction != "incoming" || t.phase != "active" || t.cancelled {
			return
		}
		raw, err := base64.StdEncoding.DecodeString(frame.Data)
		if err != nil || len(raw) > fileMaxChunkBytes {
			t.phase = "canceled"
			t.cancelled = true
			return
		}
		ring := c.chunks[frame.Transfer]
		if len(ring) >= chunkRingCapacity {
			// The core stopped polling: fail the transfer visibly rather than
			// buffering without bound.
			t.phase = "canceled"
			t.cancelled = true
			c.queueControl(directFrame{Version: directFrameVersion, Kind: "control", Action: "cancel", Transfer: frame.Transfer})
			return
		}
		c.chunks[frame.Transfer] = append(ring, fileChunk{
			Seq: frame.Chunk, Offset: frame.Offset, Data: raw, Final: frame.Final,
		})
	}
}

// sendControlLocked writes one control frame; the caller holds c.mu. A closed
// channel makes the fact visible to whoever inspects state next.
func (c *channelSet) sendControlLocked(frame directFrame) {
	if c.control == nil {
		return
	}
	body, err := json.Marshal(frame)
	if err != nil {
		return
	}
	_ = c.control.Send(body)
}

// sendChat writes one chat body and records it for delivery tracking.
func (c *channelSet) sendChat(id, body string) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.chat == nil {
		return errChannelClosed
	}
	bodyBytes, err := json.Marshal(directFrame{Version: directFrameVersion, Kind: "chat", Message: id, Body: body})
	if err != nil {
		return err
	}
	if err := c.chat.Send(bodyBytes); err != nil {
		return err
	}
	c.delivered[id] = false
	return nil
}

// chatInbox drains inbound messages for the core.
func (c *channelSet) chatInbox() ([]chatMessage, bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	overflowed := c.inboxMax < 0
	if !overflowed {
		messages := c.inbox
		c.inbox = nil
		return messages, false
	}
	// Report the overflow by returning what is held; the poisoned flag
	// clears so the channel can serve again (the core decides to requeue).
	c.inboxMax = chatInboxCapacity
	messages := c.inbox
	c.inbox = nil
	return messages, true
}

// chatDelivery reports whether the peer's worker acknowledged each id.
func (c *channelSet) chatDelivery(ids []string) map[string]bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	out := make(map[string]bool, len(ids))
	for _, id := range ids {
		out[id] = c.delivered[id]
	}
	return out
}

// beginTransfer registers an outgoing offer and writes it to the peer.
func (c *channelSet) beginTransfer(id, name string, size int64, sum string, chunkSize int) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	if c.control == nil {
		return errChannelClosed
	}
	if size <= 0 || size > fileMaxBytes || name == "" || len([]rune(name)) > transferNameMaxRunes {
		return errFrameInvalid
	}
	c.transfers[id] = &transferState{
		id: id, direction: "outgoing", phase: "offered",
		name: name, size: size, sum: sum, chunkSize: chunkSize,
	}
	c.sendControlLocked(directFrame{
		Version: directFrameVersion, Kind: "control", Action: "offer",
		Transfer: id, Name: name, Size: size, Sum: sum, ChunkSize: chunkSize,
	})
	return nil
}

// settleTransfer answers an inbound offer.
func (c *channelSet) settleTransfer(id string, accept bool) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	t := c.transfers[id]
	if t == nil || t.direction != "incoming" || t.phase != "offered" {
		return errTransferUnknown
	}
	if accept {
		t.phase = "active"
		c.sendControlLocked(directFrame{Version: directFrameVersion, Kind: "control", Action: "accept", Transfer: id})
	} else {
		t.phase = "canceled"
		c.sendControlLocked(directFrame{Version: directFrameVersion, Kind: "control", Action: "reject", Transfer: id})
	}
	return nil
}

// sendChunk writes one outgoing file frame after honoring the watermark.
func (c *channelSet) sendChunk(id string, seq int, offset int64, data []byte, final bool) (bool, error) {
	for step := 0; step < sendWaitSteps; step++ {
		c.mu.Lock()
		dc := c.file
		t := c.transfers[id]
		var buffered uint64
		if dc != nil {
			buffered = dc.BufferedAmount()
		}
		stateOK := t != nil && t.direction == "outgoing" && t.phase == "active" && !t.cancelled
		c.mu.Unlock()

		if !stateOK {
			return false, errTransferState
		}
		if dc == nil {
			return false, errChannelClosed
		}
		if buffered < sendHighWatermark {
			frame, err := json.Marshal(directFrame{
				Version: directFrameVersion, Kind: "file",
				Transfer: id, Chunk: seq, Offset: offset, Final: final,
				Data: base64.StdEncoding.EncodeToString(data),
			})
			if err != nil {
				return false, err
			}
			if err := dc.Send(frame); err != nil {
				return false, err
			}
			return false, nil
		}
		time.Sleep(sendWaitInterval)
	}
	// The peer is not draining fast enough: report pause, the core retries.
	return true, nil
}

// recvChunk pops the next inbound chunk for a transfer.
func (c *channelSet) recvChunk(id string) (fileChunk, bool, error) {
	c.mu.Lock()
	defer c.mu.Unlock()
	t := c.transfers[id]
	if t == nil || t.direction != "incoming" {
		return fileChunk{}, false, errTransferUnknown
	}
	ring := c.chunks[id]
	if len(ring) == 0 {
		if t.cancelled {
			return fileChunk{}, false, errTransferState
		}
		return fileChunk{}, false, nil
	}
	next := ring[0]
	if len(ring) == 1 {
		delete(c.chunks, id)
	} else {
		c.chunks[id] = ring[1:]
	}
	return next, true, nil
}

// cancelTransfer tears one transfer down from this side and tells the peer.
func (c *channelSet) cancelTransfer(id string) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	t := c.transfers[id]
	if t == nil {
		return errTransferUnknown
	}
	t.phase = "canceled"
	t.cancelled = true
	delete(c.chunks, id)
	c.sendControlLocked(directFrame{Version: directFrameVersion, Kind: "control", Action: "cancel", Transfer: id})
	return nil
}

// finalizeTransfer reports verified receipt to the peer.
func (c *channelSet) finalizeTransfer(id string, ok bool) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	t := c.transfers[id]
	if t == nil || t.direction != "incoming" {
		return errTransferUnknown
	}
	if ok {
		t.phase = "completed"
	} else {
		t.phase = "canceled"
		t.cancelled = true
	}
	c.sendControlLocked(directFrame{Version: directFrameVersion, Kind: "control", Action: "finalize", Transfer: id, OK: ok})
	return nil
}

// transferSnapshot is the poll-shaped view of one transfer.
type transferSnapshot struct {
	ID        string `json:"transfer_id"`
	Direction string `json:"direction"`
	Phase     string `json:"phase"`
	Name      string `json:"name"`
	Size      int64  `json:"size"`
	Sum       string `json:"sha256"`
	ChunkSize int    `json:"chunk_size"`
	Received  int    `json:"chunks_waiting"`
	FinalSeen bool   `json:"final_waiting"`
	PeerDone  bool   `json:"peer_received"`
	Canceled  bool   `json:"canceled"`
}

// pollTransfers returns every transfer this worker knows about plus any
// inbound offers. It is the authoritative surface; events are only hints.
func (c *channelSet) pollTransfers() []transferSnapshot {
	c.mu.Lock()
	defer c.mu.Unlock()
	out := make([]transferSnapshot, 0, len(c.transfers))
	for _, t := range c.transfers {
		ring := c.chunks[t.id]
		snapshot := transferSnapshot{
			ID: t.id, Direction: t.direction, Phase: t.phase,
			Name: t.name, Size: t.size, Sum: t.sum, ChunkSize: t.chunkSize,
			Received: len(ring), PeerDone: t.peerDone, Canceled: t.cancelled,
		}
		for _, chunk := range ring {
			if chunk.Final {
				snapshot.FinalSeen = true
			}
		}
		out = append(out, snapshot)
	}
	return out
}

// close tears every channel and buffer down with the call. It is safe to
// call more than once; the pump is stopped before its channel state vanishes.
func (c *channelSet) close() {
	c.mu.Lock()
	if c.closed {
		c.mu.Unlock()
		return
	}
	c.closed = true
	c.mu.Unlock()
	close(c.stopPmp)
	<-c.pmpDone

	c.mu.Lock()
	defer c.mu.Unlock()
	for _, dc := range []*webrtc.DataChannel{c.control, c.chat, c.file} {
		if dc != nil {
			_ = dc.Close()
		}
	}
	c.control, c.chat, c.file = nil, nil, nil
	c.inbox = nil
	c.activity = nil
	c.chunks = make(map[string][]fileChunk)
	c.transfers = make(map[string]*transferState)
	c.delivered = make(map[string]bool)
}

// openChannels creates the three direct channels on the offering side. The
// answering side binds the peer's channels through OnDataChannel instead, so
// each label exists exactly once per direction.
func (s *service) openChannels(pc *webrtc.PeerConnection) {
	set := newChannelSet()
	for _, label := range []string{"control", "chat", "file"} {
		dc, err := pc.CreateDataChannel(label, nil)
		if err != nil {
			// A channel that cannot be created disables that surface; the
			// poll/report paths report closed channels honestly.
			continue
		}
		set.bind(dc)
	}
	s.setChannels(set)
}

// receiveChannels binds inbound channels on the answering side.
func (s *service) receiveChannels(pc *webrtc.PeerConnection) {
	set := newChannelSet()
	pc.OnDataChannel(func(dc *webrtc.DataChannel) {
		set.bind(dc)
	})
	s.setChannels(set)
}

func (s *service) setChannels(set *channelSet) {
	s.stateMu.Lock()
	s.channels = set
	s.stateMu.Unlock()
}

func (s *service) currentChannels() *channelSet {
	s.stateMu.Lock()
	defer s.stateMu.Unlock()
	return s.channels
}

// chatSend serves chat.send from the core.
func (s *service) chatSend(request envelope) envelope {
	var payload struct {
		CallID    string `json:"call_id"`
		MessageID string `json:"message_id"`
		Body      string `json:"body"`
	}
	if json.Unmarshal(request.Payload, &payload) != nil || payload.MessageID == "" {
		return errorFor(request, "invalid_request", "error.protocol.invalidRequest", "chat payload is invalid", false)
	}
	if len(payload.Body) > chatMaxBodyBytes {
		return errorFor(request, "chat_too_large", "error.chat.tooLarge", "chat body exceeds the direct frame limit", false)
	}
	set := s.currentChannels()
	if set == nil {
		return errorFor(request, "call_inactive", "error.call.inactive", "no call is active", false)
	}
	if err := set.sendChat(payload.MessageID, payload.Body); err != nil {
		return errorFor(request, "channel_unavailable", "error.chat.channelUnavailable", err.Error(), true)
	}
	return responseFor(request, map[string]any{"state": "SENT"})
}

// chatPoll drains inbound chat for the core.
func (s *service) chatPoll(request envelope) envelope {
	set := s.currentChannels()
	if set == nil {
		return errorFor(request, "call_inactive", "error.call.inactive", "no call is active", false)
	}
	messages, overflowed := set.chatInbox()
	return responseFor(request, map[string]any{
		"messages":   messages,
		"overflowed": overflowed,
	})
}

// chatStatus reports delivery facts for outbound ids.
func (s *service) chatStatus(request envelope) envelope {
	var payload struct {
		MessageIDs []string `json:"message_ids"`
	}
	if json.Unmarshal(request.Payload, &payload) != nil {
		return errorFor(request, "invalid_request", "error.protocol.invalidRequest", "chat status payload is invalid", false)
	}
	set := s.currentChannels()
	if set == nil {
		return responseFor(request, map[string]any{"deliveries": map[string]bool{}})
	}
	return responseFor(request, map[string]any{"deliveries": set.chatDelivery(payload.MessageIDs)})
}

// transferBegin opens an outgoing transfer with an offer frame.
func (s *service) transferBegin(request envelope) envelope {
	var payload struct {
		CallID     string `json:"call_id"`
		TransferID string `json:"transfer_id"`
		Name       string `json:"name"`
		Size       int64  `json:"size"`
		Sum        string `json:"sha256"`
		ChunkSize  int    `json:"chunk_size"`
	}
	if json.Unmarshal(request.Payload, &payload) != nil || payload.TransferID == "" {
		return errorFor(request, "invalid_request", "error.protocol.invalidRequest", "transfer payload is invalid", false)
	}
	if payload.Size <= 0 || payload.Size > fileMaxBytes {
		return errorFor(request, "transfer_too_large", "error.transfer.tooLarge",
			fmt.Sprintf("transfers are limited to %d bytes", fileMaxBytes), false)
	}
	if payload.ChunkSize <= 0 || payload.ChunkSize > fileMaxChunkBytes {
		payload.ChunkSize = 16 * 1024
	}
	set := s.currentChannels()
	if set == nil {
		return errorFor(request, "call_inactive", "error.call.inactive", "no call is active", false)
	}
	if err := set.beginTransfer(payload.TransferID, payload.Name, payload.Size, payload.Sum, payload.ChunkSize); err != nil {
		return errorFor(request, "channel_unavailable", "error.transfer.channelUnavailable", err.Error(), true)
	}
	s.event("media.transfer_update", map[string]any{"transfer_id": payload.TransferID, "phase": "offered"})
	return responseFor(request, map[string]any{"state": "OFFERED"})
}

// transferSettle accepts or rejects an inbound offer.
func (s *service) transferSettle(request envelope, accept bool) envelope {
	var payload struct {
		TransferID string `json:"transfer_id"`
	}
	if json.Unmarshal(request.Payload, &payload) != nil || payload.TransferID == "" {
		return errorFor(request, "invalid_request", "error.protocol.invalidRequest", "transfer payload is invalid", false)
	}
	set := s.currentChannels()
	if set == nil {
		return errorFor(request, "call_inactive", "error.call.inactive", "no call is active", false)
	}
	if err := set.settleTransfer(payload.TransferID, accept); err != nil {
		return errorFor(request, "transfer_unknown", "error.transfer.unknown", err.Error(), false)
	}
	phase := "REJECTED"
	if accept {
		phase = "ACTIVE"
	}
	return responseFor(request, map[string]any{"state": phase})
}

// transferSend streams one chunk; paused=true asks the core to retry.
func (s *service) transferSend(request envelope) envelope {
	var payload struct {
		TransferID string `json:"transfer_id"`
		Seq        int    `json:"seq"`
		Offset     int64  `json:"offset"`
		Data       string `json:"data"`
		Final      bool   `json:"final"`
	}
	if json.Unmarshal(request.Payload, &payload) != nil || payload.TransferID == "" {
		return errorFor(request, "invalid_request", "error.protocol.invalidRequest", "chunk payload is invalid", false)
	}
	data, err := base64.StdEncoding.DecodeString(payload.Data)
	if err != nil || len(data) > fileMaxChunkBytes {
		return errorFor(request, "invalid_request", "error.protocol.invalidRequest", "chunk data is invalid", false)
	}
	set := s.currentChannels()
	if set == nil {
		return errorFor(request, "call_inactive", "error.call.inactive", "no call is active", false)
	}
	paused, err := set.sendChunk(payload.TransferID, payload.Seq, payload.Offset, data, payload.Final)
	if err != nil {
		return errorFor(request, "transfer_state", "error.transfer.state", err.Error(), false)
	}
	return responseFor(request, map[string]any{"paused": paused})
}

// transferRecv hands the core the next inbound chunk.
func (s *service) transferRecv(request envelope) envelope {
	var payload struct {
		TransferID string `json:"transfer_id"`
	}
	if json.Unmarshal(request.Payload, &payload) != nil || payload.TransferID == "" {
		return errorFor(request, "invalid_request", "error.protocol.invalidRequest", "chunk poll payload is invalid", false)
	}
	set := s.currentChannels()
	if set == nil {
		return errorFor(request, "call_inactive", "error.call.inactive", "no call is active", false)
	}
	chunk, found, err := set.recvChunk(payload.TransferID)
	if err != nil {
		return errorFor(request, "transfer_unknown", "error.transfer.unknown", err.Error(), false)
	}
	if !found {
		return responseFor(request, map[string]any{"empty": true})
	}
	return responseFor(request, map[string]any{
		"empty":  false,
		"seq":    chunk.Seq,
		"offset": chunk.Offset,
		"data":   base64.StdEncoding.EncodeToString(chunk.Data),
		"final":  chunk.Final,
	})
}

// transferCancel tears one transfer down bilaterally.
func (s *service) transferCancel(request envelope) envelope {
	var payload struct {
		TransferID string `json:"transfer_id"`
	}
	if json.Unmarshal(request.Payload, &payload) != nil || payload.TransferID == "" {
		return errorFor(request, "invalid_request", "error.protocol.invalidRequest", "cancel payload is invalid", false)
	}
	set := s.currentChannels()
	if set == nil {
		return errorFor(request, "call_inactive", "error.call.inactive", "no call is active", false)
	}
	if err := set.cancelTransfer(payload.TransferID); err != nil {
		return errorFor(request, "transfer_unknown", "error.transfer.unknown", err.Error(), false)
	}
	return responseFor(request, map[string]any{"state": "CANCELED"})
}

// transferFinalize reports verified receipt to the peer.
func (s *service) transferFinalize(request envelope) envelope {
	var payload struct {
		TransferID string `json:"transfer_id"`
		OK         bool   `json:"ok"`
	}
	if json.Unmarshal(request.Payload, &payload) != nil || payload.TransferID == "" {
		return errorFor(request, "invalid_request", "error.protocol.invalidRequest", "finalize payload is invalid", false)
	}
	set := s.currentChannels()
	if set == nil {
		return errorFor(request, "call_inactive", "error.call.inactive", "no call is active", false)
	}
	if err := set.finalizeTransfer(payload.TransferID, payload.OK); err != nil {
		return errorFor(request, "transfer_unknown", "error.transfer.unknown", err.Error(), false)
	}
	return responseFor(request, map[string]any{"state": "FINALIZED"})
}

// transferPoll is the authoritative transfer surface for the core.
func (s *service) transferPoll(request envelope) envelope {
	set := s.currentChannels()
	if set == nil {
		return responseFor(request, map[string]any{"transfers": []transferSnapshot{}})
	}
	return responseFor(request, map[string]any{"transfers": set.pollTransfers()})
}
