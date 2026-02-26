import { StreamRenderer } from './stream';
import { useStreamStatus } from './streams';

import { cn } from '@shadcn/lib/utils';

// ── Styles (JetBrains Islands) ──────────────────────────────────────────

/// Shared island panel style — dark surface, subtle border, soft shadow.
const island = cn(
    "border border-[#393b40] rounded-lg",
    "shadow-[0_1px_3px_rgba(0,0,0,0.3),0_4px_12px_rgba(0,0,0,0.15)]",
    "bg-[#2b2d30]");

// ── App ─────────────────────────────────────────────────────────────────

/// Pure viewer shell.  Stream lifecycle is fully server-managed — the
/// frontend just renders two well-known stream IDs and polls for
/// availability to show/hide the YouTube Music island.
export function App() {
    const { hasYouTubeMusic } = useStreamStatus();

    return (
        <div className="flex flex-col flex-1 gap-2 p-2">
            <div className="flex flex-row flex-1 gap-2">
                <div className={"flex flex-col flex-3 gap-2"}>
                    {/* Top Row: Marquee Banner */}
                    <div className={cn(island, "flex flex-col flex-1 overflow-hidden")}>
                    </div>
                    {/* Top Row 2: Unused */}
                    <div className={cn("flex flex-col flex-1 overflow-hidden")}>
                    </div>
                    {/* Main Content */}
                    <div className={cn(island, "aspect-16-10 p-0.5 overflow-hidden")}>
                        <div className="w-full rounded-md overflow-clip">
                            <StreamRenderer streamId="main" />
                        </div>
                    </div>
                    {/* Bottom Row: Unused */}
                    <div className={cn(island, "flex flex-col flex-1 overflow-hidden")}>
                    </div>
                </div>
                <div className={cn(island, "flex-1 p-6 flex flex-col gap-3")}>
                    <span className="text-[#bcc0cc]">Hi, I'm Nekomaru OwO</span>
                </div>
            </div>
            <div className={cn(island, "h-16 overflow-hidden")}>
                {hasYouTubeMusic ? (
                    <StreamRenderer streamId="youtube-music" />
                ) : (
                    <Placeholder>YouTube Music not detected</Placeholder>
                )}
            </div>
        </div>
    );
}

// ── Sub-components ──────────────────────────────────────────────────────

/// Centered placeholder shown when a stream is unavailable.
function Placeholder({ children }: { children: React.ReactNode }) {
    return (
        <div className={cn(
            "flex items-center justify-center",
            "min-h-50 text-[#6f737a] text-sm")}>
            {children}
        </div>
    );
}
