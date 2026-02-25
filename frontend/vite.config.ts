import * as path from "node:path";
import * as vite from "vite";
import react from "@vitejs/plugin-react-swc";
import tailwindcss from "@tailwindcss/vite";

export default vite.defineConfig({
    root: "src",
    plugins: [
        react(),
        tailwindcss(),
    ],
    resolve: {
        alias: {
            "@":
                path.resolve(__dirname, "src"),
            "@shadcn":
                path.resolve(__dirname, "3rdparty", "shadcn"),
        },
    },
});
