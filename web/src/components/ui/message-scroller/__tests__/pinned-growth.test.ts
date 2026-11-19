import { afterEach, beforeEach, expect, test, vi } from 'vite-plus/test'
import { mount } from '@vue/test-utils'
import { defineComponent, h } from 'vue'
import {
  MessageScrollerContent,
  MessageScrollerItem,
  MessageScrollerProvider,
  MessageScrollerViewport,
} from '..'

const observers: (() => void)[] = []
const frame = () => new Promise<void>((resolve) => requestAnimationFrame(() => resolve()))
let view: ReturnType<typeof mount>
beforeEach(() => {
  vi.stubGlobal(
    'ResizeObserver',
    class {
      constructor(callback: () => void) {
        observers.push(callback)
      }
      observe() {}
      disconnect() {}
    },
  )
})
afterEach(() => {
  view?.unmount()
  observers.length = 0
  vi.unstubAllGlobals()
})

/** A viewport with fake geometry: 300 px tall, `height` px of content. */
async function mountScroller() {
  const Scroller = defineComponent(
    () => () =>
      h(MessageScrollerProvider, { autoScroll: true, defaultScrollPosition: 'end' }, () =>
        h(MessageScrollerViewport, null, () =>
          h(MessageScrollerContent, null, () =>
            h(MessageScrollerItem, { messageId: 'a' }, () => 'hi'),
          ),
        ),
      ),
  )
  view = mount(Scroller, { attachTo: document.body })
  const viewport = view.get('[data-slot="message-scroller-viewport"]').element as HTMLElement
  const geometry = { height: 500 }
  Object.defineProperty(viewport, 'clientHeight', { configurable: true, value: 300 })
  Object.defineProperty(viewport, 'scrollHeight', {
    configurable: true,
    get: () => geometry.height,
  })
  viewport.scrollTo = ((options: ScrollToOptions) => {
    viewport.scrollTop = options.top ?? 0
  }) as typeof viewport.scrollTo
  await frame()
  return { viewport, geometry }
}
const grow = async (geometry: { height: number }, height: number) => {
  geometry.height = height
  observers.forEach((callback) => callback())
  await frame()
  await frame()
}

test('a reader at the bottom follows growth after a wheel event that did not scroll', async () => {
  const { viewport, geometry } = await mountScroller()
  viewport.scrollTop = 200
  viewport.dispatchEvent(new Event('scroll'))
  viewport.dispatchEvent(new Event('wheel'))
  await grow(geometry, 700)
  expect(viewport.scrollTop).toBe(400)
})

test('a reader who scrolled up keeps their place when the content grows', async () => {
  const { viewport, geometry } = await mountScroller()
  viewport.scrollTop = 50
  viewport.dispatchEvent(new Event('scroll'))
  viewport.dispatchEvent(new Event('wheel'))
  await grow(geometry, 700)
  expect(viewport.scrollTop).toBe(50)
})
