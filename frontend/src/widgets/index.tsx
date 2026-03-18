import { useState, useEffect, useMemo } from "react";

import { InfoIcon } from "lucide-react";
import { DynamicIcon } from "lucide-react/dynamic";

import Grid from "@/components/grid";
import { LiveWidget } from "@/widgets/common";

// ── Clock ────────────────────────────────────────────────────────────────────
export function ClockWidget() {
    return <Grid columns="1fr 1fr" gap="2">
        <ClockComponent timeZone="Asia/Shanghai" label="UTC+8 Beijing" />
        <ClockComponent timeZone="America/New_York" label="UTC-4 New York" variant="secondary" />
    </Grid>;
}

function ClockComponent({ timeZone, label, variant = undefined }: {
    timeZone: string,
    label: string,
    variant?: "secondary",
}) {
    const format = useMemo(() => new Intl.DateTimeFormat("en-GB", {
        timeZone,
        hour: "2-digit",
        minute: "2-digit",
        hour12: false,
    }), [timeZone]);

    const [time, setTime] = useState(() => format.format(new Date()));

    useEffect(() => {
        const id = setInterval(() => setTime(format.format(new Date())), 60 * 1000);
        return () => clearInterval(id);
    }, [format]);

    type ClockHour = 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12;
    const hours = (parseInt(time.split(":")[0] ?? "0", 10) % 12 || 12) as ClockHour;
    const iconName = `clock-${hours}` as const;

    return <LiveWidget
        name={label}
        icon={<DynamicIcon name={iconName} size={40} />}
        className={variant === "secondary" ? "opacity-50" : ""}>
        <span className="text-2xl">{time}</span>
    </LiveWidget>;
}

// ── Mode ─────────────────────────────────────────────────────────────────────

/// Shows the current live mode derived from the auto-selector's `@mode` tag.
/// Reads `$liveMode` from the string store passed in as props.
export function LiveModeWidget({ strings }: { strings: Record<string, string> }) {
    /// Display labels and icons for each mode value.
    const MODE_MAP = {
        unknown: { label: "—", icon: "activity" },
        code: { label: "Coding", icon: "code" },
        game: { label: "Gaming", icon: "gamepad" },
        sing: { label: "Singing", icon: "music" },
        chat: { label: "Chatting", icon: "message-circle" },
        brb: { label: "BRB", icon: "coffee" },
    } as const;

    const mode = (
        strings.$liveMode
            && MODE_MAP[strings.$liveMode as keyof typeof MODE_MAP])
            || MODE_MAP.unknown;

    return <LiveWidget
        name="Live Mode"
        icon={<DynamicIcon name={mode.icon} size={36} />}>
        <span className="text-md">{mode.label}</span>
    </LiveWidget>;
}

// ── Capture ──────────────────────────────────────────────────────────────────

/// Large widget showing the current capture target and mode (AUTO/LOCKED).
/// Reads `$captureMode` and `$captureInfo` computed strings.
export function CaptureWidget({ strings }: { strings: Record<string, string> }) {
    const captureMode =
        strings.$captureMode?.toUpperCase() ?? "—";
    const captureInfo =
        strings.$captureInfo ?? "";

    return <LiveWidget
        name={`Capture Mode - ${captureMode}`}
        icon={<DynamicIcon name="monitor" size={36} />}>
        <span className="text-sm truncate">
            {captureInfo && <> {captureInfo}</>}
        </span>
    </LiveWidget>;
}

// ── About ────────────────────────────────────────────────────────────────────

/// Large widget anchored to the bottom of the SidePanel.
/// Shows the revision timestamp and credits.
export function AboutWidget({ strings }: { strings: Record<string, string> }) {
    const timestamp = strings.$timestamp ?? "";
    return <div className="text-xs">
        <LiveWidget
            name="Nekomaru LiveUI v2"
            icon={<InfoIcon size={30} />}>
            <div className="text-xs opacity-50">
                {timestamp && <span>Build {timestamp.slice(0, 16)}</span>}
            </div>
        </LiveWidget>
        <div className="px-2">
            <span className="opacity-75">Designed & Created by Nekomaru@pku.edu.cn</span>
            <span className="opacity-50">Coauthored by Claude Code</span>
        </div>
    </div>;
}
