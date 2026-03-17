/**
 * GPU-accelerated chroma-key renderer.
 *
 * Replaces pixels matching a target color with transparency using a WebGL2
 * fragment shader.  The entire pipeline stays on the GPU — the video frame
 * is uploaded as a texture, the shader computes per-pixel alpha via Chebyshev
 * distance + smoothstep, and the result composites against whatever CSS
 * background is behind the canvas.
 */

// ── Shaders ──────────────────────────────────────────────────────────────────

/// Fullscreen triangle from gl_VertexID — no vertex buffer needed.
const VERT_SRC = /* glsl */ `#version 300 es
out vec2 v_uv;
void main() {
    // Vertices: (-1,-1), (3,-1), (-1,3) — covers the full clip quad.
    vec2 pos = vec2(
        float((gl_VertexID & 1) << 2) - 1.0,
        float((gl_VertexID & 2) << 1) - 1.0);
    v_uv = pos * 0.5 + 0.5;
    v_uv.y = 1.0 - v_uv.y;  // flip Y for video texture coordinates
    gl_Position = vec4(pos, 0.0, 1.0);
}
`

/// Chroma-key fragment shader.  Pixels near u_keyColor get low alpha
/// (smoothstep falloff), everything else stays fully opaque.
const FRAG_SRC = /* glsl */ `#version 300 es
precision mediump float;
in vec2 v_uv;
out vec4 fragColor;
uniform sampler2D u_texture;
uniform vec3 u_keyColor;    // target color in [0,1] RGB
uniform float u_threshold;  // distance at which alpha reaches 1.0
void main() {
    vec4 color = texture(u_texture, v_uv);
    // Chebyshev distance — max per-channel delta from the key color.
    vec3 diff = abs(color.rgb - u_keyColor);
    float dist = max(max(diff.r, diff.g), diff.b);
    float alpha = smoothstep(0.0, u_threshold, dist);
    fragColor = vec4(color.rgb, alpha);
}
`

// ── Renderer ─────────────────────────────────────────────────────────────────

/// Default threshold in normalised [0,1] space.  ~30/255 ≈ 0.118.
const DEFAULT_THRESHOLD = 30.0 / 255.0

export class ChromaKeyRenderer {
    private gl: WebGL2RenderingContext
    private program: WebGLProgram
    private texture: WebGLTexture
    private vao: WebGLVertexArrayObject
    private canvas: HTMLCanvasElement

    /**
     * @param canvas  Target canvas element (will be bound to a WebGL2 context).
     * @param keyColor  RGB tuple in [0,255] to key out.
     * @param threshold  Distance (normalised) at which alpha reaches 1.0.
     */
    constructor(canvas: HTMLCanvasElement, keyColor: [number, number, number], threshold = DEFAULT_THRESHOLD) {
        this.canvas = canvas

        const gl = canvas.getContext("webgl2", {
            alpha: true,
            premultipliedAlpha: false,
        })
        if (!gl) throw new Error("ChromaKeyRenderer: WebGL2 not available")
        this.gl = gl

        // ── Compile & link ───────────────────────────────────────────────
        this.program = createProgram(gl, VERT_SRC, FRAG_SRC)

        // ── Empty VAO (vertex positions computed from gl_VertexID) ────────
        const vao = gl.createVertexArray()
        if (!vao) throw new Error("ChromaKeyRenderer: failed to create VAO")
        this.vao = vao
        gl.bindVertexArray(this.vao)
        gl.bindVertexArray(null)

        // ── Texture for video frames ─────────────────────────────────────
        const texture = gl.createTexture()
        if (!texture) throw new Error("ChromaKeyRenderer: failed to create texture")
        this.texture = texture
        gl.bindTexture(gl.TEXTURE_2D, this.texture)
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE)
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE)
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR)
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR)

        // ── Upload uniforms ──────────────────────────────────────────────
        // biome-ignore lint/correctness/useHookAtTopLevel: gl.useProgram is a WebGL method, not a React hook
        gl.useProgram(this.program)
        gl.uniform1i(gl.getUniformLocation(this.program, "u_texture"), 0)
        gl.uniform3f(
            gl.getUniformLocation(this.program, "u_keyColor"),
            keyColor[0] / 255, keyColor[1] / 255, keyColor[2] / 255)
        gl.uniform1f(gl.getUniformLocation(this.program, "u_threshold"), threshold)
    }

    /** Render a decoded video frame with chroma-key applied. Closes the frame. */
    render(frame: VideoFrame): void {
        const gl = this.gl

        // Resize canvas + viewport when video dimensions change.
        if (this.canvas.width !== frame.displayWidth || this.canvas.height !== frame.displayHeight) {
            this.canvas.width = frame.displayWidth
            this.canvas.height = frame.displayHeight
            gl.viewport(0, 0, frame.displayWidth, frame.displayHeight)
            console.log("ChromaKeyRenderer: Resized to %dx%d", frame.displayWidth, frame.displayHeight)
        }

        // Upload frame as texture.
        gl.bindTexture(gl.TEXTURE_2D, this.texture)
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, frame)

        // Frame data is now on the GPU — release the VideoFrame immediately.
        frame.close()

        // Draw fullscreen triangle.
        // biome-ignore lint/correctness/useHookAtTopLevel: gl.useProgram is a WebGL method, not a React hook
        gl.useProgram(this.program)
        gl.bindVertexArray(this.vao)
        gl.drawArrays(gl.TRIANGLES, 0, 3)
    }

    dispose(): void {
        const gl = this.gl
        gl.deleteTexture(this.texture)
        gl.deleteVertexArray(this.vao)
        gl.deleteProgram(this.program)
    }
}

// ── WebGL helpers ────────────────────────────────────────────────────────────

function compileShader(gl: WebGL2RenderingContext, type: number, source: string): WebGLShader {
    const shader = gl.createShader(type)
    if (!shader) throw new Error(`Failed to create shader (type=${type})`)
    gl.shaderSource(shader, source)
    gl.compileShader(shader)
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
        const log = gl.getShaderInfoLog(shader)
        gl.deleteShader(shader)
        throw new Error(`Shader compile error: ${log}`)
    }
    return shader
}

function createProgram(gl: WebGL2RenderingContext, vertSrc: string, fragSrc: string): WebGLProgram {
    const vert = compileShader(gl, gl.VERTEX_SHADER, vertSrc)
    const frag = compileShader(gl, gl.FRAGMENT_SHADER, fragSrc)
    const program = gl.createProgram()
    if (!program) throw new Error("Failed to create WebGL program")
    gl.attachShader(program, vert)
    gl.attachShader(program, frag)
    gl.linkProgram(program)
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
        const log = gl.getProgramInfoLog(program)
        gl.deleteProgram(program)
        throw new Error(`Program link error: ${log}`)
    }
    // Shaders are linked — no longer needed as standalone objects.
    gl.deleteShader(vert)
    gl.deleteShader(frag)
    return program
}

/**
 * Parse a CSS hex color string (#RRGGBB) into an [R, G, B] tuple in [0,255].
 * Throws on invalid format.
 */
export function parseHexColor(hex: string): [number, number, number] {
    const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex)
    if (!m) throw new Error(`Invalid hex color: ${hex}`)
    const [, r, g, b] = m as unknown as [string, string, string, string]
    return [parseInt(r, 16), parseInt(g, 16), parseInt(b, 16)]
}
