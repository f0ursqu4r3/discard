import { beforeEach, expect, test, vi } from 'vite-plus/test'
import { flushPromises, mount } from '@vue/test-utils'
import Composer from '../Composer.vue'
import MessageRow from '../MessageRow.vue'
import AttachmentCard from '../AttachmentCard.vue'
import type { AttachmentDto } from '@/bridge/types'
const api = vi.hoisted(() => ({
  chooseAttachments: vi.fn(),
  importAttachmentFiles: vi.fn(),
  attachmentPreview: vi.fn(),
  getAgents: vi.fn(),
  getProjectMembers: vi.fn(),
  runAction: vi.fn(),
  contextMenu: vi.fn(),
}))
vi.mock('@/bridge/api', () => api)
const image: AttachmentDto = {
  id: 'image',
  name: 'screenshot.png',
  mediaType: 'image/png',
  size: 200,
  text: null,
}
const document: AttachmentDto = {
  id: 'document',
  name: 'notes.pdf',
  mediaType: 'application/pdf',
  size: 500,
  text: 'Extracted document text',
}
const props = { channelId: 'c1', archived: false, busy: false, running: false, channel: 'general' }
beforeEach(() => {
  vi.clearAllMocks()
  api.chooseAttachments.mockResolvedValue([image, document])
  api.importAttachmentFiles.mockResolvedValue([image])
  api.attachmentPreview.mockResolvedValue('data:image/png;base64,aW1hZ2U=')
})
test('picks files, shows compact previews, removes a file, and sends attachments without text', async () => {
  const view = mount(Composer, { props })
  expect(view.get('[data-ui-id="composer-send"]').attributes('disabled')).toBeDefined()
  await view.get('[aria-label="Attach files"]').trigger('click')
  await flushPromises()
  expect(api.chooseAttachments).toHaveBeenCalledOnce()
  expect(view.findAllComponents(AttachmentCard)).toHaveLength(2)
  expect(view.get('img').attributes('src')).toContain('data:image/png;')
  expect(view.text()).toContain('notes.pdf')
  await view.get('[aria-label="Remove screenshot.png"]').trigger('click')
  expect(view.findAllComponents(AttachmentCard)).toHaveLength(1)
  await view.get('[data-ui-id="composer-send"]').trigger('click')
  expect(view.emitted('send')).toEqual([['', [document]]])
  expect(view.findAllComponents(AttachmentCard)).toHaveLength(0)
  view.unmount()
})
test('controlled attachment drafts remain until the parent confirms a successful send', async () => {
  const view = mount(Composer, {
    props: { ...props, draft: 'Review these', attachments: [image, document] },
  })
  await view.get('textarea').trigger('keydown', { key: 'Enter' })
  expect(view.emitted('send')).toEqual([['Review these', [image, document]]])
  expect(view.findAllComponents(AttachmentCard)).toHaveLength(2)
  expect(view.emitted('update:attachments')).toBeUndefined()
  await view.setProps({ attachments: [] })
  expect(view.findAllComponents(AttachmentCard)).toHaveLength(0)
  view.unmount()
})
test('failed file selection keeps the draft and reports the error', async () => {
  api.chooseAttachments.mockRejectedValue(new Error('Unsupported file'))
  const view = mount(Composer, { props: { ...props, draft: 'Keep this', attachments: [document] } })
  await view.get('[aria-label="Attach files"]').trigger('click')
  await flushPromises()
  expect(view.get('[role="alert"]').text()).toContain('Unsupported file')
  expect(view.findAllComponents(AttachmentCard)).toHaveLength(1)
  expect(view.get('textarea').element.value).toBe('Keep this')
  view.unmount()
})
test('busy and archived composers disable attachment changes', async () => {
  const view = mount(Composer, { props: { ...props, attachments: [document], busy: true } })
  expect(view.get('[aria-label="Attach files"]').attributes('disabled')).toBeDefined()
  expect(view.get('[aria-label="Remove notes.pdf"]').attributes('disabled')).toBeDefined()
  await view.setProps({ busy: false, archived: true })
  expect(view.get('[aria-label="Attach files"]').attributes('disabled')).toBeDefined()
  view.unmount()
})
function transferEvent(type: string, files: File[], extra = {}) {
  const event = new Event(type, { bubbles: true, cancelable: true })
  Object.defineProperty(event, type === 'paste' ? 'clipboardData' : 'dataTransfer', {
    value: { types: ['Files'], files, items: [], ...extra },
  })
  return event
}
const clipboardImage = () => new File(['image'], 'clipboard.png', { type: 'image/png' })
test('drop highlights the composer and imports files into the existing previews', async () => {
  const view = mount(Composer, { props })
  const dropped = new File(['notes'], 'notes.txt', { type: 'text/plain' })
  await flushPromises()
  view.element.dispatchEvent(transferEvent('dragenter', [dropped]))
  await flushPromises()
  expect(view.get('[role="status"]').text()).toBe('Drop files to attach')
  const event = transferEvent('drop', [dropped])
  await flushPromises()
  view.element.dispatchEvent(event)
  await flushPromises()
  expect(event.defaultPrevented).toBe(true)
  expect(api.importAttachmentFiles).toHaveBeenCalledWith([dropped])
  expect(view.find('[role="status"]').exists()).toBe(false)
  expect(view.findAllComponents(AttachmentCard)).toHaveLength(1)
  view.unmount()
})
test('the main panel accepts drops above the composer and owns the full-panel highlight', async () => {
  const panel = globalThis.document.createElement('section')
  const header = globalThis.document.createElement('header')
  const messages = globalThis.document.createElement('div')
  panel.append(header, messages)
  globalThis.document.body.append(panel)
  const view = mount(Composer, { attachTo: panel, props: { ...props, dropTarget: panel } })
  await flushPromises()
  const file = clipboardImage()
  header.dispatchEvent(transferEvent('dragenter', [file]))
  await flushPromises()
  expect(panel.querySelector('[role="status"]')?.parentElement).toBe(panel)
  messages.dispatchEvent(transferEvent('drop', [file]))
  await flushPromises()
  expect(api.importAttachmentFiles).toHaveBeenCalledExactlyOnceWith([file])
  expect(panel.querySelector('[role="status"]')).toBeNull()
  expect(view.findAllComponents(AttachmentCard)).toHaveLength(1)
  // A drop inside the composer bubbles to the same single listener.
  view.get('textarea').element.dispatchEvent(transferEvent('drop', [file]))
  await flushPromises()
  expect(api.importAttachmentFiles).toHaveBeenCalledTimes(2)
  view.unmount()
  header.dispatchEvent(transferEvent('drop', [file]))
  await flushPromises()
  expect(api.importAttachmentFiles).toHaveBeenCalledTimes(2)
  panel.remove()
})
test('nested drag targets keep the highlight until the drag leaves the composer', async () => {
  const view = mount(Composer, { props })
  const file = clipboardImage()
  await flushPromises()
  view.element.dispatchEvent(transferEvent('dragenter', [file]))
  view.get('textarea').element.dispatchEvent(transferEvent('dragenter', [file]))
  view.get('textarea').element.dispatchEvent(transferEvent('dragleave', [file]))
  await flushPromises()
  expect(view.find('[role="status"]').exists()).toBe(true)
  await flushPromises()
  view.element.dispatchEvent(transferEvent('dragleave', [file]))
  await flushPromises()
  expect(view.find('[role="status"]').exists()).toBe(false)
  view.unmount()
})
test('pasting an image adds a preview without changing the text draft', async () => {
  const view = mount(Composer, { props: { ...props, draft: 'Keep this text' } })
  const file = clipboardImage()
  const event = transferEvent('paste', [file])
  view.get('textarea').element.dispatchEvent(event)
  await flushPromises()
  expect(event.defaultPrevented).toBe(true)
  expect(api.importAttachmentFiles).toHaveBeenCalledWith([file])
  expect(view.findAllComponents(AttachmentCard)).toHaveLength(1)
  expect(view.get('textarea').element.value).toBe('Keep this text')
  view.unmount()
})
test('ordinary text paste keeps the native paste behavior', async () => {
  const view = mount(Composer, { props })
  const event = transferEvent('paste', [], { types: ['text/plain'] })
  view.get('textarea').element.dispatchEvent(event)
  await flushPromises()
  expect(event.defaultPrevented).toBe(false)
  expect(api.importAttachmentFiles).not.toHaveBeenCalled()
  view.unmount()
})
test.each([{ busy: true }, { archived: true }])(
  'drop and paste respect disabled state %o',
  async (state) => {
    const view = mount(Composer, { props: { ...props, ...state } })
    await flushPromises()
    view.element.dispatchEvent(transferEvent('drop', [clipboardImage()]))
    view.get('textarea').element.dispatchEvent(transferEvent('paste', [clipboardImage()]))
    await flushPromises()
    expect(api.importAttachmentFiles).not.toHaveBeenCalled()
    view.unmount()
  },
)
test('rejects a drop that exceeds the existing draft limit before importing', async () => {
  const view = mount(Composer, {
    props: {
      ...props,
      attachments: Array.from({ length: 8 }, (_, i) => ({ ...image, id: String(i) })),
    },
  })
  await flushPromises()
  view.element.dispatchEvent(transferEvent('drop', [clipboardImage()]))
  await flushPromises()
  expect(api.importAttachmentFiles).not.toHaveBeenCalled()
  expect(view.get('[role="alert"]').text()).toContain('up to 8 files')
  expect(view.findAllComponents(AttachmentCard)).toHaveLength(8)
  view.unmount()
})
test('failed imports preserve existing attachment and text drafts', async () => {
  api.importAttachmentFiles.mockRejectedValueOnce(new Error('Unsupported file'))
  const view = mount(Composer, { props: { ...props, attachments: [document], draft: 'Keep this' } })
  await flushPromises()
  view.element.dispatchEvent(transferEvent('drop', [clipboardImage()]))
  await flushPromises()
  expect(view.get('[role="alert"]').text()).toContain('Unsupported file')
  expect(view.findAllComponents(AttachmentCard)).toHaveLength(1)
  expect(view.get('textarea').element.value).toBe('Keep this')
  view.unmount()
})
test('rejects folders without importing them as empty files', async () => {
  const view = mount(Composer, { props })
  await flushPromises()
  view.element.dispatchEvent(
    transferEvent('drop', [clipboardImage()], {
      items: [{ webkitGetAsEntry: () => ({ isDirectory: true }) }],
    }),
  )
  await flushPromises()
  expect(api.importAttachmentFiles).not.toHaveBeenCalled()
  expect(view.get('[role="alert"]').text()).toContain('Folders are not supported')
  view.unmount()
})
test('ignores repeat imports and discards results after the composer closes', async () => {
  let finish!: (files: AttachmentDto[]) => void
  api.importAttachmentFiles.mockReturnValueOnce(
    new Promise((resolve) => {
      finish = resolve
    }),
  )
  const view = mount(Composer, { props })
  await flushPromises()
  view.element.dispatchEvent(transferEvent('drop', [clipboardImage()]))
  view.get('textarea').element.dispatchEvent(transferEvent('paste', [clipboardImage()]))
  await flushPromises()
  expect(api.importAttachmentFiles).toHaveBeenCalledOnce()
  view.unmount()
  finish([image])
  await flushPromises()
  expect(view.emitted('update:attachments')).toBeUndefined()
})
test('a reopened message renders its attachment card', async () => {
  const view = mount(MessageRow, {
    props: {
      channelId: 'channel',
      message: {
        id: 'saved',
        role: 'user',
        continuesGroup: false,
        blocks: [{ type: 'attachment', attachment: document }],
      },
    },
  })
  expect(view.getComponent(AttachmentCard).text()).toContain('notes.pdf')
  expect(view.find('[aria-label="Remove notes.pdf"]').exists()).toBe(false)
  await view.get('[aria-label="Preview notes.pdf"]').trigger('click')
  await flushPromises()
  expect(globalThis.document.body.textContent).toContain('Extracted document text')
  view.unmount()
})
