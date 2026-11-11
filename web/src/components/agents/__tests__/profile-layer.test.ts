import { defineComponent } from 'vue'
import { flushPromises, mount } from '@vue/test-utils'
import { expect, test, vi } from 'vite-plus/test'
import AgentProfileDialog from '../AgentProfileDialog.vue'
import { Sheet, SheetContent, SheetDescription, SheetTitle } from '@/components/ui/sheet'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => []) }))

test('the full profile opens above an existing sheet on every open', async () => {
  const profile = mount(AgentProfileDialog, {
    props: { agentId: null },
    attachTo: document.body,
  })
  await flushPromises()
  const sheet = mount(
    defineComponent({
      components: { Sheet, SheetContent, SheetDescription, SheetTitle },
      template: `<Sheet :open="true"><SheetContent>
        <SheetTitle>Profile</SheetTitle>
        <SheetDescription>Short profile</SheetDescription>
      </SheetContent></Sheet>`,
    }),
    { attachTo: document.body },
  )
  try {
    await flushPromises()
    for (let opening = 0; opening < 2; opening++) {
      await profile.setProps({ agentId: 'a' })
      await flushPromises()
      const sheetContent = document.querySelector('[data-slot="sheet-content"]')!
      const profileOverlay = document.querySelector('[data-slot="dialog-overlay"]')!
      const profileContent = document.querySelector('[data-slot="dialog-content"]')!
      // These layers share a z-index, so their portal order sets the paint order.
      expect(
        sheetContent.compareDocumentPosition(profileOverlay) & Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy()
      expect(
        profileOverlay.compareDocumentPosition(profileContent) & Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy()
      await profile.setProps({ agentId: null })
      await flushPromises()
      expect(document.querySelector('[data-slot="sheet-content"]')).not.toBeNull()
    }
  } finally {
    profile.unmount()
    sheet.unmount()
  }
})
