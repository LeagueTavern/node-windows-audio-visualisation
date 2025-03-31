import { createRequire } from "module"
const require = createRequire(import.meta.url)
export const {
  getAllOutputDevices,
  getDefaultOutputDevice,
  AudioMonitor,
} = require("./index.js")
