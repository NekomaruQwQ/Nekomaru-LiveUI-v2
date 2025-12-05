import { defineConfig } from "vite";
import preact from '@preact/preset-vite'

export default defineConfig({
    root: "frontend",
    plugins: [
        preact(),
    ],
    server: {
        port: 9688,
        strictPort: true,
    },
});
