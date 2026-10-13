import type { FaceState } from "../../src/lib/face/expression-state";
export interface BallPlacement { side: "left" | "right"; offsetY: number }
export interface BallState { face: FaceState; open: boolean; spin: number }
export interface BallSnapshot extends BallState { revision: number }
export type BallDrag = "start" | "move" | "end" | "cancel";
