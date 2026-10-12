import { mount } from "svelte";
import App from "./lib/App.svelte";
import "./styles.css";

const app = document.querySelector("#app");
if (!app) {
  throw new Error("missing #app root");
}

export default mount(App, { target: app });