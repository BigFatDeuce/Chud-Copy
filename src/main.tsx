import { createRoot } from "react-dom/client";
import "@fontsource-variable/sora";
import "@fontsource-variable/jetbrains-mono";
import "./styles/app.css";
import { App } from "./App";

createRoot(document.getElementById("root")!).render(<App />);
