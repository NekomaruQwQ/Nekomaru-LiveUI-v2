import { css } from '@emotion/css';
import { useEffect, useState } from 'react';

import { api } from './api';
import { StreamRenderer } from './streamRenderer';

/// Default capture resolution (matches RTX 5090 / existing constants).
const DEFAULT_WIDTH = 1920;
const DEFAULT_HEIGHT = 1200;

/// Expected shape of window info from the enumerate-windows crate.
/// Adjust field names if the Rust crate uses a different schema.
interface WindowInfo {
    hwnd: number;
    title: string;
}

// ── Styles ──────────────────────────────────────────────────────────────

const card = css({
    borderColor: 'rgba(255, 255, 255, 0.2)',
    borderWidth: 1,
    borderStyle: 'solid',
    borderRadius: 12,
    boxShadow: [
        'rgba(128,128,128,0.5) 2px 4px 16px',
        'inset rgba(255, 255, 255, 0.1) 1px 2px 4px',
    ].join(', '),
    backgroundColor: 'rgba(255, 255, 255, 0.5)',
    backdropFilter: 'blur(24px) brightness(0.95)',
});

// ── App ─────────────────────────────────────────────────────────────────

export function App() {
    /// Active stream ID, or null when no capture is running.
    const [streamId, setStreamId] = useState<string | null>(null);
    /// Capturable windows shown in the picker, or null when picker is closed.
    const [windows, setWindows] = useState<WindowInfo[] | null>(null);
    /// True during the initial "do we already have a stream?" check.
    const [loading, setLoading] = useState(true);

    // On mount, check for an existing running stream (e.g. after page reload).
    useEffect(() => {
        (async () => {
            try {
                const res = await api.index.$get();
                if (res.ok) {
                    const streams = await res.json();
                    const running = streams.find((s) => s.status === "running");
                    if (running) {
                        setStreamId(running.id);
                    }
                }
            } catch (e) {
                console.error("Failed to list streams:", e);
            } finally {
                setLoading(false);
            }
        })();
    }, []);

    /// Fetch the list of capturable windows and show the picker.
    async function loadWindows() {
        try {
            const res = await api.windows.$get();
            if (!res.ok) return;
            const data = await res.json() as unknown as WindowInfo[];
            setWindows(data);
        } catch (e) {
            console.error("Failed to enumerate windows:", e);
        }
    }

    /// Create a capture stream for the selected window.
    async function startCapture(win: WindowInfo) {
        try {
            const res = await api.index.$post({
                json: {
                    hwnd: String(win.hwnd),
                    width: DEFAULT_WIDTH,
                    height: DEFAULT_HEIGHT,
                },
            });
            if (!res.ok) return;
            const { id } = await res.json();
            setStreamId(id);
            setWindows(null);
        } catch (e) {
            console.error("Failed to create stream:", e);
        }
    }

    /// Stop the active stream and return to idle.
    async function stopCapture() {
        if (!streamId) return;
        try {
            await api[":id"].$delete({ param: { id: streamId } });
        } catch (e) {
            console.error("Failed to destroy stream:", e);
        }
        setStreamId(null);
    }

    // ── Render ───────────────────────────────────────────────────────────

    return (
        <div className={css({
            padding: '32px 32px',
            display: 'flex',
            flexDirection: 'column',
            flex: 1,
            gap: 16,
        })}>
            <div className={css({
                display: 'flex',
                flexDirection: 'row',
                gap: 24,
            })}>
                <div className={[
                    card,
                    css({
                        flex: 3,
                        padding: 16,
                        overflow: 'hidden',
                    }),
                ].join(' ')}>
                    {loading ? (
                        <Placeholder>Connecting...</Placeholder>
                    ) : streamId ? (
                        <StreamRenderer streamId={streamId} />
                    ) : windows ? (
                        <WindowPicker
                            windows={windows}
                            onSelect={startCapture}
                            onCancel={() => setWindows(null)}
                        />
                    ) : (
                        <Placeholder>
                            <button type="button" onClick={loadWindows} className={pillButton}>
                                Start Capture
                            </button>
                        </Placeholder>
                    )}
                </div>
                <div className={[
                    card,
                    css({
                        flex: 1,
                        padding: 24,
                        display: 'flex',
                        flexDirection: 'column',
                        gap: 12,
                    }),
                ].join(' ')}>
                    Hi, I'm Nekomaru OwO
                    {streamId && (
                        <button type="button" onClick={stopCapture} className={pillButton}>
                            Stop Capture
                        </button>
                    )}
                </div>
            </div>
            <div className={[
                card,
                css({
                    flex: 1,
                    padding: 8,
                }),
            ].join(' ')}>
            </div>
        </div>
    );
}

// ── Sub-components ──────────────────────────────────────────────────────

/// Centered placeholder shown when no stream is active.
function Placeholder({ children }: { children: React.ReactNode }) {
    return (
        <div className={css({
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            minHeight: 200,
            color: 'rgba(0, 0, 0, 0.5)',
            fontSize: 14,
        })}>
            {children}
        </div>
    );
}

/// List of capturable windows — user clicks one to start capturing it.
function WindowPicker({ windows, onSelect, onCancel }: {
    windows: WindowInfo[];
    onSelect: (w: WindowInfo) => void;
    onCancel: () => void;
}) {
    return (
        <div className={css({
            display: 'flex',
            flexDirection: 'column',
            gap: 2,
            maxHeight: 400,
            overflowY: 'auto',
        })}>
            <div className={css({
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'space-between',
                padding: '8px 12px',
            })}>
                <span className={css({ fontWeight: 600, fontSize: 14 })}>
                    Select a window to capture
                </span>
                <button type="button" onClick={onCancel} className={pillButton}>
                    Cancel
                </button>
            </div>
            {windows.map((w) => (
                <button
                    type="button"
                    key={w.hwnd}
                    onClick={() => onSelect(w)}
                    className={css({
                        display: 'block',
                        width: '100%',
                        padding: '10px 12px',
                        border: 'none',
                        borderRadius: 8,
                        backgroundColor: 'transparent',
                        textAlign: 'left',
                        fontSize: 13,
                        cursor: 'pointer',
                        ':hover': {
                            backgroundColor: 'rgba(0, 0, 0, 0.06)',
                        },
                    })}
                >
                    {w.title || `Window ${w.hwnd}`}
                </button>
            ))}
        </div>
    );
}

// ── Shared styles ───────────────────────────────────────────────────────

const pillButton = css({
    padding: '6px 16px',
    border: '1px solid rgba(0, 0, 0, 0.15)',
    borderRadius: 8,
    backgroundColor: 'rgba(255, 255, 255, 0.7)',
    fontSize: 13,
    cursor: 'pointer',
    ':hover': {
        backgroundColor: 'rgba(255, 255, 255, 0.9)',
    },
});
