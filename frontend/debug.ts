/**
 * Runtime debug control for frontend components
 *
 * Toggle these flags to enable/disable console output for each module.
 * These are runtime flags, so you can toggle them without rebuilding.
 *
 * # Usage in Code
 *
 * ```typescript
 * import { DEBUG } from './debug';
 *
 * if (DEBUG.decoder) {
 *     console.log('[Decoder] debug message');
 * }
 * ```
 *
 * # Developer Workflow
 *
 * Option 1: Edit this file and reload the page
 * Option 2: Toggle in browser console (no reload needed):
 *
 * ```javascript
 * window.__DEBUG__.decoder = true;   // Enable decoder logs
 * window.__DEBUG__.renderer = true;  // Enable renderer logs
 * window.__DEBUG__.stream = true;    // Enable stream loop logs
 * ```
 */

export const DEBUG = {
    /** Enable debug output for H.264 decoder (decoder.ts) */
    decoder: false,

    /** Enable debug output for video renderer component (renderer.tsx) */
    renderer: false,

    /** Enable debug output for stream loop (renderer.tsx stream logic) */
    stream: false,
};

/**
 * Expose DEBUG object to global scope for easy console access
 * Access via: window.__DEBUG__
 */
declare global {
    interface Window {
        __DEBUG__: typeof DEBUG;
    }
}

(globalThis as any).__DEBUG__ = DEBUG;
