import * as path from "node:path";
import * as vite from "vite";
import react from "@vitejs/plugin-react-swc";
import tailwindcss from "@tailwindcss/vite";

export default vite.defineConfig({
    root: __dirname,
    plugins: [
        react(),
        tailwindcss(),
    ],
    resolve: {
        alias: {
            "@": path.resolve(__dirname, "src"),
        },
    },
    server: {
        port: Number(process.env.LIVE_PORT),

        // Allow any host to connect to the dev server.  This is necessary when running
        // the frontend on another pc.
        allowedHosts: true,
    },
});
