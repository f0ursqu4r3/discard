import { onScopeDispose, readonly, ref, type Ref } from 'vue'

export const SPINNER_FRAMES = '⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏'
export const SPINNER_INTERVAL_MS = 90
export const STATIC_FRAME = '⠿'
const frame = ref(SPINNER_FRAMES[0]!)
let subscribers = 0
let index = 0
let timer: ReturnType<typeof setInterval> | undefined
let media: MediaQueryList | undefined

function stop() {
  if (timer !== undefined) clearInterval(timer)
  timer = undefined
}
function syncMotion() {
  stop()
  index = 0
  frame.value = media?.matches ? STATIC_FRAME : SPINNER_FRAMES[0]!
  if (!media?.matches) {
    timer = setInterval(() => {
      index = (index + 1) % SPINNER_FRAMES.length
      frame.value = SPINNER_FRAMES[index]!
    }, SPINNER_INTERVAL_MS)
  }
}

export function useSpinnerFrame(): Readonly<Ref<string>> {
  if (subscribers++ === 0) {
    media =
      typeof matchMedia === 'function' ? matchMedia('(prefers-reduced-motion: reduce)') : undefined
    media?.addEventListener('change', syncMotion)
    syncMotion()
  }
  onScopeDispose(() => {
    if (--subscribers === 0) {
      stop()
      media?.removeEventListener('change', syncMotion)
      media = undefined
    }
  })
  return readonly(frame)
}
