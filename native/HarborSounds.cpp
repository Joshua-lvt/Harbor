// The vendored implementation compiles exactly once, in this translation
// unit; every other file gets the declarations only.
#define MA_IMPLEMENTATION
#include <miniaudio.h>

#include "HarborSounds.h"

#include <QTimer>
#include <QtGlobal>

#include <cmath>

namespace {

constexpr ma_uint32 kSampleRate = 44100;
// Every tone fades in and out so the blip never clicks.
constexpr double kFadeSeconds = 0.008;

} // namespace

// The audio callback only walks the shared sample buffer; the GUI thread is
// the sole writer (play restarts the cursor under the same mutex).
void HarborSounds::dataCallback(ma_device *device, void *output,
                                const void *input, ma_uint32 frameCount)
{
    Q_UNUSED(input);
    auto *sounds = static_cast<HarborSounds *>(device->pUserData);
    auto *out = static_cast<ma_int16 *>(output);
    if (sounds == nullptr) {
        for (ma_uint32 i = 0; i < frameCount; ++i)
            out[i] = 0;
        return;
    }
    QMutexLocker locker(&sounds->m_mutex);
    const size_t total = sounds->m_samples.size();
    for (ma_uint32 i = 0; i < frameCount; ++i) {
        if (sounds->m_cursor >= total) {
            out[i] = 0;
            continue;
        }
        out[i] = sounds->m_samples[sounds->m_cursor++];
    }
}

HarborSounds::HarborSounds(QObject *parent)
    : QObject(parent)
{
    // Honest capability: no usable audio stack, no sounds — the adapter
    // reports unavailable instead of silently swallowing plays.
    m_contextOk = ma_context_init(nullptr, 0, nullptr, &m_context) == MA_SUCCESS;
}

HarborSounds::~HarborSounds()
{
    stop();
    if (m_deviceInit)
        ma_device_uninit(&m_device);
    if (m_contextOk)
        ma_context_uninit(&m_context);
}

bool HarborSounds::available() const
{
    return m_contextOk;
}

void HarborSounds::renderTone(double fromHz, double toHz, double seconds,
                              double amplitude)
{
    const size_t start = m_samples.size();
    const size_t frames = static_cast<size_t>(seconds * kSampleRate);
    m_samples.resize(start + frames);
    const double fadeFrames = kFadeSeconds * kSampleRate;
    for (size_t i = 0; i < frames; ++i) {
        const double t = static_cast<double>(i) / kSampleRate;
        const double progress = static_cast<double>(i) / static_cast<double>(frames);
        // Exponential frequency glide: a rising one reads as the water-drop
        // blip the settings copy promises, a flat one as a soft ping.
        const double freq = fromHz * std::pow(toHz / fromHz, progress);
        const double phase = 2.0 * M_PI * freq * t;
        double envelope = 1.0;
        if (static_cast<double>(i) < fadeFrames)
            envelope = static_cast<double>(i) / fadeFrames;
        else if (static_cast<double>(frames - i) < fadeFrames)
            envelope = static_cast<double>(frames - i) / fadeFrames;
        m_samples[start + i] = static_cast<ma_int16>(
            amplitude * envelope * std::sin(phase) * 32767.0);
    }
}

void HarborSounds::appendSilence(double seconds)
{
    const size_t frames = static_cast<size_t>(seconds * kSampleRate);
    m_samples.insert(m_samples.end(), frames, 0);
}

void HarborSounds::stop()
{
    if (m_deviceInit && ma_device_is_started(&m_device))
        ma_device_stop(&m_device);
}

void HarborSounds::play(const QString &kind)
{
    if (!m_contextOk)
        return;

    // Short synthesized blips (s16 mono). Kind chooses the shape: a rising
    // drop for chat, a soft ping for presence, a double blip for calls.
    m_samples.clear();
    if (kind == QLatin1String("presence")) {
        renderTone(494.0, 494.0, 0.10, 0.22);
    } else if (kind == QLatin1String("call")) {
        renderTone(660.0, 660.0, 0.09, 0.25);
        appendSilence(0.06);
        renderTone(660.0, 660.0, 0.09, 0.25);
    } else {
        // "message" and any unknown kind: the quiet rising drop.
        renderTone(587.0, 880.0, 0.14, 0.25);
    }

    QMutexLocker locker(&m_mutex);
    m_cursor = 0;
    locker.unlock();

    if (!m_deviceInit) {
        ma_device_config config = ma_device_config_init(ma_device_type_playback);
        config.playback.format = ma_format_s16;
        config.playback.channels = 1;
        config.sampleRate = kSampleRate;
        config.dataCallback = dataCallback;
        config.pUserData = this;
        if (ma_device_init(&m_context, &config, &m_device) != MA_SUCCESS)
            return;
        m_deviceInit = true;
    }

    if (ma_device_start(&m_device) != MA_SUCCESS)
        return;

    // The callback trails into silence after the buffer ends; stop the
    // device shortly after the tone's natural end so no stream stays open.
    const int durationMs = static_cast<int>(
        (static_cast<double>(m_samples.size()) / kSampleRate) * 1000.0);
    QTimer::singleShot(durationMs + 150, this, &HarborSounds::stop);
}
