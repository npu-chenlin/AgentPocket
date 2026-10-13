import type { AgentPocketApi } from "../electron/preload";
import type { BallApi } from "../electron/preload/ball";
declare global {
  interface Window {
    agentpocket: AgentPocketApi;
    agentpocketBall: BallApi;
  }
}
export {};
