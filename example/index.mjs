import { AudioMonitor, getDefaultOutputDevice } from "../index.mjs"

const chunkSize = 2048
const spectrumLength = 50
const spectrumBands = 16

const defaultDevice = getDefaultOutputDevice()
const audio = new AudioMonitor()

audio.setDevice(defaultDevice.id)
audio.start(chunkSize)

setInterval(() => {
  const spectrum = audio
    .getSpectrum(spectrumBands)
    .map((v) => v * (spectrumLength - 1) + 1)
  console.clear()
  console.log(`Device: ${defaultDevice.name}`)
  console.log(Array.from({ length: spectrumLength }, () => "-").join(""))
  console.log(
    spectrum
      .map((v) => Array.from({ length: Math.floor(v) }, () => "|").join(""))
      .join("\n")
  )
}, 1e3 / 20)
