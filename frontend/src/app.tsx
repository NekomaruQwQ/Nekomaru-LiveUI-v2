import { AudioStream } from "@/audio";
import { StreamRenderer } from "@/video";
import { useStreamStatus } from "@/streams";
import { useStrings } from "@/strings";
import Marquee from "@/components/marquee";
import Grid from "@/components/grid";
import { ClockWidget, LiveModeWidget, CaptureWidget, AboutWidget } from "./widgets";
import { KpmMeter } from "@/kpm";

/// Pure viewer shell.  Stream lifecycle is fully server-managed — the
/// frontend just renders two well-known stream IDs and polls for
/// availability to show/hide the YouTube Music island.
export function App() {
    const { hasYouTubeMusic } = useStreamStatus();
    const strings = useStrings();

    return (
        <Grid rows="1fr 60px" gap="2" className="w-screen h-screen p-2">
            {/* Global audio stream (renders nothing visible) */}
            <AudioStream />
            {/* Everything other than the YouTube Music island */}
            <Grid columns="1fr 3fr 40px" gap="2">
                {/* Side Column: User Info */}
                <SidePanel />
                {/* Main Column: Marquee + Main Stream */}
                <Grid rows="auto 1fr" gap="2">
                    {/* Top Row: Marquee Banner */}
                    <div className="island">
                        {strings.marquee && <Marquee text={strings.marquee} />}
                    </div>
                    <div className="island flex-col flex-1">
                        <div className="flex-1 rounded-md items-center justify-center bg-[#1d1d1d]!">
                            <StreamRenderer streamId="main" />
                        </div>
                    </div>
                </Grid>
                {/* Side Column: User Info */}
                <ActionPanel />
            </Grid>
            {/* Bottom Row: YouTube Music (conditionally rendered) */}
            <div className="island h-15 items-center justify-center">
                {hasYouTubeMusic && <StreamRenderer streamId="youtube-music" chromaKey="#212121" />}
            </div>
        </Grid>
    );
}


function SidePanel() {
    const strings = useStrings();
    return <div className="flex! w-full h-full flex-col gap-2">
        <div className="island px-2 py-1.5">
            <ClockWidget />
        </div>
        <div className="island px-2 py-1.5">
            <LiveModeWidget strings={strings} />
            <CaptureWidget strings={strings} />
        </div>
        <div className="island px-3 py-2 flex-1">
            <pre className="font-sans font-light whitespace-pre-wrap wrap-break-word">
                {strings.message}
            </pre>
        </div>
        <div className="island px-2 py-1.5">
            <AboutWidget strings={strings} />
        </div>
    </div>;
}

function ActionPanel() {
    return <div className="island p-2 flex! w-full h-full flex-col">
        <KpmMeter />
    </div>;
}
