import test from "ava"

import { getAllOutputDevices, getDefaultOutputDevice } from ".."

test("Output devices getter", (t) => {
  t.true(Array.isArray(getAllOutputDevices()))
})

test("Default output device getter", (t) => {
  t.is(typeof getDefaultOutputDevice(), "object")
})
