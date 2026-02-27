import { StreamRenderer } from '@/stream';
import { useStreamStatus } from '@/streams';
import { useStrings } from '@/strings';
import Marquee from '@/components/marquee';
import Grid from '@/components/grid';

/// Pure viewer shell.  Stream lifecycle is fully server-managed — the
/// frontend just renders two well-known stream IDs and polls for
/// availability to show/hide the YouTube Music island.
export function App() {
    const { hasYouTubeMusic } = useStreamStatus();
    const strings = useStrings();

    return (
        <Grid rows="1fr 60px" gap="2" className="w-screen h-screen p-2">
            {/* Everything other than the YouTube Music island */}
            <Grid columns="3fr 1fr" gap="2">
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
                <div className="island p-2">
                    <SidePanel />
                </div>
            </Grid>
            {/* Bottom Row: YouTube Music (conditionally rendered) */}
            <div className="island h-15 items-center justify-center bg-[#141414]!">
                {hasYouTubeMusic && <StreamRenderer streamId="youtube-music" />}
            </div>
        </Grid>
    );
}


function SidePanel() {
    return <div className={"w-full h-full flex-col flex-1 gap-3"}>
        <span>Hi, I'm Nekomaru OwO</span>
        {/* Add more user info or controls here */}
    </div>;
}
