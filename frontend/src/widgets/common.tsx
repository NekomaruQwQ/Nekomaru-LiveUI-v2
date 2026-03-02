import type { ReactNode } from "react";

type LiveWidgetProps = {
    name: ReactNode;
    /// Optional icon rendered to the left of the label+content stack.
    /// The parent decides what to pass — SVG component, <img>, emoji, etc.
    icon?: ReactNode;
    className?: string;
    children: ReactNode;
};

/// Two-row status indicator with an optional icon presenter.
///
/// Layout: icon (left, vertically centered) | label + content stack (right).
/// Purely presentational — the parent supplies the icon and dynamic content
/// (e.g. from the string store).
export function LiveWidget({ name, icon, className, children }: LiveWidgetProps) {
    return (
        <div className={`flex! flex-row items-center gap-1 ${className}`}>
            {icon && <div className="flex! size-10 items-center justify-center shrink-0 opacity-50">{icon}</div>}
            <div className="flex! flex-col">
                <div className="pl-0.5 text-xs opacity-75">{name}</div>
                <div>{children}</div>
            </div>
        </div>
    );
}


