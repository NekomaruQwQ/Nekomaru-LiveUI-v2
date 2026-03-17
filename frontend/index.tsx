import { createRoot } from "react-dom/client";

import { App } from "./src/app";

const app = document.getElementById("app");
if (!app) throw new Error("Missing #app element");

createRoot(app).render(<App />);
