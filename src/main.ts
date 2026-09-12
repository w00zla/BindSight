import { createApp } from "vue";
import VueKonva from "vue-konva";
import App from "./App.vue";
import { forwardConsoleToLog } from "./logging";
import "./styles/base.css";

forwardConsoleToLog();

const app = createApp(App);
// A component error names its component and hook in the log; in a release
// build Vue would only console.error the bare error.
app.config.errorHandler = (err, _instance, info) => console.error(`vue error (${info})`, err);
app.use(VueKonva);
app.mount("#app");
