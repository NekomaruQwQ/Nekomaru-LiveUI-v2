import { useState, useEffect, useMemo } from "react";
import { LiveWidget } from "./common";
import Grid from "@/components/grid";

import { DynamicIcon } from "lucide-react/dynamic";

interface ClockWidgetProps {
    timeZone: string,
    label: string,
    variant?: "secondary",
}

function ClockComponent({ timeZone, label, variant = undefined }: ClockWidgetProps) {
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

export function ClockWidget() {
    return <Grid columns="1fr 1fr" gap="2">
        <ClockComponent timeZone="Asia/Shanghai" label="UTC+8 Beijing" />
        <ClockComponent timeZone="America/New_York" label="UTC-4 New York" variant="secondary" />
    </Grid>;
}
