// A/V timestamp sync controller.
//
// Audio arrives faster than video (no encoding step), so the sync strategy
// is: audio paces itself to the video clock.  Audio chunks are held until
// their timestamp is within `SYNC_THRESHOLD_US` of the latest displayed
// video frame's timestamp.
//
// Shared between:
//   - StreamRenderer (reports video timestamps via reportVideoTimestamp)
//   - AudioStream    (checks shouldRelease before posting chunks to worklet)

/// How far ahead of the video clock we allow audio to play (microseconds).
/// 20ms threshold accounts for measurement jitter and polling intervals.
const SYNC_THRESHOLD_US = 20_000n;

class AVSyncController {
    /// Timestamp of the most recently displayed video frame (microseconds).
    private latestVideoTimestampUs: bigint = 0n;

    /// Called by the video renderer when a frame is displayed.
    reportVideoTimestamp(timestampUs: bigint): void {
        this.latestVideoTimestampUs = timestampUs;
    }

    /// Check whether an audio chunk with the given timestamp should be
    /// released to the worklet for playback.
    ///
    /// Returns true if the audio timestamp is at or behind the video clock
    /// (plus the jitter threshold).  When no video timestamp has been
    /// reported yet (value is 0), all audio is released — this prevents
    /// audio from being permanently held if the video stream hasn't started.
    shouldRelease(audioTimestampUs: bigint): boolean {
        if (this.latestVideoTimestampUs === 0n) return true;
        return audioTimestampUs <= this.latestVideoTimestampUs + SYNC_THRESHOLD_US;
    }
}

/// Global singleton — shared between audio and video components.
export const avSync = new AVSyncController();
