import type { CSSProperties, ReactNode } from 'react';

type GridProps = {
    gap?: number | string;
    className?: string;
    style?: CSSProperties;
    children: ReactNode;
} & (
    | { rows: string; columns?: never }
    | { rows?: never; columns: string });

export default function Grid({
    rows,
    columns,
    gap,
    className,
    style,
    children,
}: GridProps) {
    const gridStyle: CSSProperties = {
        display: 'grid',
        // The defined axis uses the provided track list; the cross axis fills all space.
        gridTemplateRows: rows ?? '1fr',
        gridTemplateColumns: columns ?? '1fr',
        ...style,
    };

    return (
        <div className={`gap-${gap} ${className}`} style={gridStyle}>
            {children}
        </div>
    );
};
