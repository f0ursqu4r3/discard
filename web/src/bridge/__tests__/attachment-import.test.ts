import { beforeEach, expect, test, vi } from 'vite-plus/test'
import { importAttachmentFiles } from '../api'
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke }))
beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue([])
})
test('transfers browser file bytes and names without requesting filesystem access', async () => {
  await importAttachmentFiles([new File(['hello'], 'notes.txt', { type: 'text/plain' })])
  expect(invoke).toHaveBeenCalledWith('import_attachments', {
    files: [{ name: 'notes.txt', data: 'aGVsbG8=' }],
  })
})
test('rejects file and batch limits before reading or invoking native code', async () => {
  const file = new File(['a'], 'a.png', { type: 'image/png' })
  Object.defineProperty(file, 'size', { value: 5 * 1024 * 1024 + 1, configurable: true })
  await expect(importAttachmentFiles([file])).rejects.toThrow('5 MB')
  Object.defineProperty(file, 'size', { value: 5 * 1024 * 1024 })
  await expect(importAttachmentFiles(Array(5).fill(file))).rejects.toThrow('20 MB')
  await expect(importAttachmentFiles(Array(9).fill(file))).rejects.toThrow('8 files')
  expect(invoke).not.toHaveBeenCalled()
})
