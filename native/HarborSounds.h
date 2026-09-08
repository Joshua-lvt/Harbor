#pragma once

#include <QMutex>
#include <QObject>
#include <QString>
#include <vector>

#include <miniaudio.h>

/// Notification sound playback: short synthesized sine tones, rendered into
/// memory and played through miniaudio (vendored, public domain). No media
/// framework dependency, no asset files, and nothing plays unless the user's
/// "notification sounds" setting already allowed it — the caller decides
/// policy; this adapter only knows how to make a sound.
///
/// A failing audio stack degrades to `available == false` and silent no-ops;
/// notification cards still appear without sound.
class HarborSounds final : public QObject
{
    Q_OBJECT
    /// True while an audio output could be opened. Absence is honest: the
    /// QML side skips sounds instead of pretending they played.
    Q_PROPERTY(bool available READ available CONSTANT FINAL)

public:
    explicit HarborSounds(QObject *parent = nullptr);
    ~HarborSounds() override;

    bool available() const;

    /// Plays one short tone for a notification kind ("message", "presence",
    /// "call"). Unknown kinds fall back to the message tone. A tone already
    /// playing restarts, keeping the interface stateless for callers.
    Q_INVOKABLE void play(const QString &kind);

private:
    void renderTone(double fromHz, double toHz, double seconds, double amplitude);
    void appendSilence(double seconds);
    void stop();
    /// miniaudio callback: only walks the shared buffer; a static member so
    /// it can touch the private playback state without exposing it.
    static void dataCallback(ma_device *device, void *output, const void *input,
                             ma_uint32 frameCount);

    ma_context m_context;
    bool m_contextOk = false;
    ma_device m_device;
    bool m_deviceInit = false;
    /// Rendered tone, mono s16 at kSampleRate. Guarded with the cursor by
    /// m_mutex: the audio-thread callback reads, the GUI thread rewrites.
    std::vector<ma_int16> m_samples;
    size_t m_cursor = 0;
    QMutex m_mutex;
};
