import { render } from "solid-js/web";
import "./app/styles.css";
import App from "./app/App";
import { applyTheme, loadThemePreference } from "./app/theme";

applyTheme(loadThemePreference());
render(() => <App />, document.getElementById("root")!);
