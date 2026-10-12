import { render } from "preact";
import { App, savedTheme } from "./app";
import "./style.css";

document.documentElement.dataset.theme = savedTheme(); // before the first paint, so no flash
render(<App />, document.getElementById("app")!);
