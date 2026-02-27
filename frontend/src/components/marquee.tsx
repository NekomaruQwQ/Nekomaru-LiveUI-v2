import { useEffect, useRef, useState } from "react";

/// Pixels per second — consistent reading speed regardless of text length.
const MARQUEE_SPEED = 30;

/// Seamlessly looping horizontal text scroll.  Two identical copies of the
/// text sit side-by-side; the animation shifts by exactly one copy's width
/// (−50%), so the loop reset is invisible.  Duration is derived from the
/// rendered width so scroll speed stays constant for any text length.
export default function Marquee({ text }: { text: string }) {
    const ref = useRef<HTMLSpanElement>(null);
    const [duration, setDuration] = useState(0);

    // Derive animation duration from the rendered width of one text copy.
    // ResizeObserver re-fires automatically when the text content changes.
    useEffect(() => {
        const el = ref.current;
        if (!el) return;
        const measure = () => setDuration(el.offsetWidth / MARQUEE_SPEED);
        const observer = new ResizeObserver(measure);
        observer.observe(el);
        measure();
        return () => observer.disconnect();
    }, []);

    const item = `${text}\u2002·\u2002`;

    return (
        <div
            className="flex! overflow-visible! flex-1 flex-row marquee text-[#bcc0cc] text-sm animate-[marquee_linear_infinite]"
            style={{ animationDuration: `${duration}s` }}>
            <span ref={ref} className="shrink-0 min-w-auto">{item}</span>
            <span className="shrink-0 min-w-auto">{item}</span>
        </div>
    );
}
