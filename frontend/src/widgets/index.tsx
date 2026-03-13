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

    let hours = parseInt(time.split(":")[0]!, 10);
    hours = hours % 12 || 12; // Convert to 12-hour format, treating 0 as 12
    const iconName = `clock-${hours}`;

    return <LiveWidget
        name={label}
        icon={<DynamicIcon name={iconName as any} size={40} />}
        className={variant === "secondary" ? "opacity-50" : ""}>
        <span className="text-2xl">{time}</span>
    </LiveWidget>;
}

// ── Status (Mode + Microphone) ───────────────────────────────────────────────

/// Small widget pair: Mode (left) + Microphone (right), sharing one island.
/// Reads `mode` and `microphone` from the string store passed in as props.
export function StatusWidget({ strings }: { strings: Record<string, string> }) {
    /// Display labels and icons for each mode value.
    const MODE_MAP = {
        unknown: { label: "—", icon: "activity" },
        code: { label: "Coding", icon: "code" },
        sing: { label: "Singing", icon: "music" },
        chat: { label: "Chatting", icon: "message-circle" },
        brb: { label: "BRB", icon: "coffee" },
    } as const;

    const mode = (
        strings.$liveMode
            && MODE_MAP[strings.$liveMode as keyof typeof MODE_MAP])
            || MODE_MAP.unknown;
    const micOn = strings.microphone === "on";

    return <Grid columns="1fr 1fr" gap="2">
        <LiveWidget
            name="Live Mode"
            icon={<DynamicIcon name={mode.icon} size={36} />}>
            <span className="text-md">{mode.label}</span>
        </LiveWidget>
        <LiveWidget
            name="Microphone"
            icon={<DynamicIcon name={micOn ? "mic" : "mic-off"} size={36} />}>
            <span className="text-md">{micOn ? "On" : "Muted"}</span>
        </LiveWidget>
    </Grid>;
}

// ── Capture ──────────────────────────────────────────────────────────────────

/// Large widget showing the current capture target and mode (AUTO/LOCKED).
/// Reads `$captureMode` and `$captureWindowTitle` computed strings.
export function CaptureWidget({ strings }: { strings: Record<string, string> }) {
    const captureMode =
        strings.$captureMode?.toUpperCase() ?? "—";
    const windowTitle =
        strings.$captureWindowTitle?.split("-").slice(-1)[0]?.trim() ?? "";

    return <LiveWidget
        name={`Capture Mode - ${captureMode}`}
        icon={<DynamicIcon name="monitor" size={36} />}>
        <span className="text-sm truncate">
            {windowTitle && <> {windowTitle}</>}
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
        <div>
            <span className="opacity-75">Designed & Created by Nekomaru@pku.edu.cn</span>
            <span className="opacity-50">Coauthored by Claude Code</span>
        </div>
    </div>;
}
