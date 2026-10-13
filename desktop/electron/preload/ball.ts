import { contextBridge, ipcRenderer } from "electron";
import type { BallDrag, BallSnapshot } from "../main/ball-ipc";

const api = {
  drag: (action: BallDrag): Promise<void> => ipcRenderer.invoke("ball:drag", action),
  toggle: (): Promise<void> => ipcRenderer.invoke("ball:toggle"),
  getState: (): Promise<BallSnapshot> => ipcRenderer.invoke("ball:get-state"),
  onState: (callback: (state: BallSnapshot) => void): (() => void) => {
    const handler = (_event: unknown, state: BallSnapshot) => callback(state);
    ipcRenderer.on("ball:state", handler);
    return () => ipcRenderer.removeListener("ball:state", handler);
  },
};
export type BallApi = typeof api;
contextBridge.exposeInMainWorld("agentpocketBall", api);
