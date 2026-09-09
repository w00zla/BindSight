import { createApp } from "vue";
import VueKonva from "vue-konva";
import App from "./App.vue";
import { forwardConsoleToLog } from "./logging";
import "./styles/base.css";

forwardConsoleToLog();

const app = createApp(App);
app.use(VueKonva);
app.mount("#app");
console.info(`frontend mounted (${navigator.userAgent})`);
