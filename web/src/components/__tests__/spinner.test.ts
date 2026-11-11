import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { defineComponent, h, nextTick } from 'vue'
import { mount } from '@vue/test-utils'
import { SPINNER_FRAMES, SPINNER_INTERVAL_MS, STATIC_FRAME, useSpinnerFrame } from '../spinner'
import MemberList from '../MemberList.vue'

const hosts: ReturnType<typeof mount>[] = []
let reduced = false
let media: EventTarget
beforeEach(() => {
  vi.useFakeTimers()
  reduced = false
  media = new EventTarget()
  vi.stubGlobal('matchMedia', () => Object.assign(media, { matches: reduced }))
})
afterEach(() => {
  hosts.splice(0).forEach((host) => host.unmount())
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  vi.useRealTimers()
})
function host() {
  const view = mount(
    defineComponent({
      setup() {
        const frame = useSpinnerFrame()
        return () => h('span', frame.value)
      },
    }),
  )
  hosts.push(view)
  return view
}
test('advances one frame per interval', async () => {
  const view = host()
  expect(view.text()).toBe(SPINNER_FRAMES[0])
  vi.advanceTimersByTime(SPINNER_INTERVAL_MS)
  await nextTick()
  expect(view.text()).toBe(SPINNER_FRAMES[1])
})
test('subscribers share one timer', () => {
  const start = vi.spyOn(globalThis, 'setInterval')
  const stop = vi.spyOn(globalThis, 'clearInterval')
  host()
  host()
  expect(start).toHaveBeenCalledTimes(1)
  hosts.shift()!.unmount()
  expect(stop).not.toHaveBeenCalled()
  hosts.shift()!.unmount()
  expect(stop).toHaveBeenCalledTimes(1)
})
test('reduced motion shows the static frame and starts no timer', () => {
  reduced = true
  const start = vi.spyOn(globalThis, 'setInterval')
  expect(host().text()).toBe(STATIC_FRAME)
  expect(start).not.toHaveBeenCalled()
})
test('motion preference changes stop and restart the shared timer', async () => {
  const view = host()
  Object.assign(media, { matches: true })
  media.dispatchEvent(new Event('change'))
  await nextTick()
  expect(view.text()).toBe(STATIC_FRAME)
  expect(vi.getTimerCount()).toBe(0)
  Object.assign(media, { matches: false })
  media.dispatchEvent(new Event('change'))
  vi.advanceTimersByTime(SPINNER_INTERVAL_MS)
  await nextTick()
  expect(view.text()).toBe(SPINNER_FRAMES[1])
})
test('a working member has an accessible spinner instead of a status dot', () => {
  const view = mount(MemberList, {
    props: { agentId: 'agent-a', agent: 'Builder', status: 'working', provider: 'claude' },
  })
  hosts.push(view)
  expect(view.find('[role="img"][aria-label="Working"]').exists()).toBe(true)
  expect(view.find('[data-testid="presence-dot"]').exists()).toBe(false)
})
