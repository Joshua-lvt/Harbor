package main

import (
	"time"

	"github.com/pion/webrtc/v4"
)

// Transport sampling during a live call. Pion measures its own connection;
// this file only reduces those measurements to the bounded numeric facts the
// core may surface: the nominated ICE pair's round-trip time and the
// cumulative audio stream counters. No payloads, endpoints, or identifiers
// beyond the call id the core already owns ever leave the worker.

// statsInterval paces transport sampling while a call is live. Two seconds
// keeps the UI responsive without competing with the voice pipeline.
const statsInterval = 2 * time.Second

// callStats is one sample of the call's own transport health.
type callStats struct {
	RttMs    float64 `json:"rtt_ms"`
	Received uint32  `json:"received"`
	Lost     int32   `json:"lost"`
}

// extractCallStats reduces one Pion stats report to the bounded facts above.
// A report without a nominated pair or audio stream yet (pre-connect, or the
// first instants after answer) yields ok=false, and the tick stays silent
// instead of publishing zeros that would look like real measurements.
func extractCallStats(report webrtc.StatsReport) (callStats, bool) {
	var stats callStats
	var haveRTT, haveAudio bool
	for _, entry := range report {
		switch sample := entry.(type) {
		case webrtc.ICECandidatePairStats:
			if sample.Nominated && sample.CurrentRoundTripTime > 0 {
				stats.RttMs = sample.CurrentRoundTripTime * 1000
				haveRTT = true
			}
		case webrtc.InboundRTPStreamStats:
			if sample.Kind == "audio" {
				stats.Received = sample.PacketsReceived
				stats.Lost = sample.PacketsLost
				haveAudio = true
			}
		}
	}
	// All facts or none: a partial sample never leaks out of the worker.
	if !haveRTT || !haveAudio {
		return callStats{}, false
	}
	return stats, true
}

// startStatsTicker samples the live call's transport until stopped. At most
// one sampler exists per worker: calls are sequential inside one process.
func (s *service) startStatsTicker(callID string) {
	s.stateMu.Lock()
	if s.statsStop != nil {
		s.stateMu.Unlock()
		return
	}
	stop := make(chan struct{})
	s.statsStop = stop
	s.stateMu.Unlock()

	go func() {
		ticker := time.NewTicker(statsInterval)
		defer ticker.Stop()
		for {
			select {
			case <-stop:
				return
			case <-ticker.C:
				s.sampleCallStats(callID)
			}
		}
	}()
}

func (s *service) stopStatsTicker() {
	s.stateMu.Lock()
	stop := s.statsStop
	s.statsStop = nil
	s.stateMu.Unlock()
	if stop != nil {
		close(stop)
	}
}

func (s *service) sampleCallStats(callID string) {
	s.stateMu.Lock()
	pc := s.pc
	s.stateMu.Unlock()
	if pc == nil {
		return
	}
	stats, ok := extractCallStats(pc.GetStats())
	if !ok {
		return
	}
	// Asynchronous core-scoped fact, like the voice levels: the event bus
	// serializes it with everything else the worker publishes.
	s.event("media.call_stats", map[string]any{
		"call_id":  callID,
		"rtt_ms":   stats.RttMs,
		"received": stats.Received,
		"lost":     stats.Lost,
	})
}
