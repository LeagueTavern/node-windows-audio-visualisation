import { AudioMonitor, getDefaultOutputDevice } from "../index.mjs"

const defaultDevice = getDefaultOutputDevice()
const audio = new AudioMonitor()

audio.setDevice(defaultDevice.id)
audio.play()

const length = 50
const bands = 16
const dancy = 12

setInterval(() => {
  const spectrum = audio
    .getSpectrum(bands, dancy, 1024)
    .map((v) => v * (length - 1) + 1)
  console.clear()
  console.log(`Device: ${defaultDevice.name}`)
  console.log(Array.from({ length }, () => "-").join(""))
  console.log(
    spectrum
      .map((v) => Array.from({ length: Math.floor(v) }, () => "|").join(""))
      .join("\n")
  )
}, 1e3 / 20)
