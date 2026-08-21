import { createApp } from "vue";
import App from "./App.vue";
import router from "./router/index.ts";
import "./styles/colors.css";
import "./styles/variables.css";
import "./styles/global.css";

const app = createApp(App);
app.use(router);
app.mount("#app");
