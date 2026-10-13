import { mount } from "svelte";

import BallApp from "./lib/BallApp.svelte";
import "./ball.css";

const root = document.querySelector("#ball");
if (!root) {
  throw new Error("missing #ball root");
}

export default mount(BallApp, { target: root });
