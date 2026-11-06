import { mount } from '@vue/test-utils'
import { expect, test, vi } from 'vite-plus/test'
vi.mock('../bridge/api', () => ({
  decideApproval: vi.fn(),
  confirmResolve: vi.fn(),
  archiveConfirm: vi.fn(),
}))
import ApprovalCard from './ApprovalCard.vue'
import Notices from './Notices.vue'
const banners = {
  reviewCount: 1,
  problemLines: ['problem'],
  confirm: 'Quit?',
  archive: { message: 'Archive?' },
  safeMode: false,
}
test('stale approvals have no decision buttons', () => {
  const card = mount(ApprovalCard, {
    props: {
      channelId: 'channel',
      prompt: { approvalId: 'a', tool: 'shell', summary: 'run', view: 'stale' },
    },
  })
  expect(card.find('button').exists()).toBe(false)
  expect(card.get('[data-protected="true"]').text()).toContain('Expired')
})
test('confirm notice has both decisions and protected surfaces remain present', () => {
  const notices = mount(Notices, { props: { banners, notice: null, archived: false } })
  const surfaces = notices.findAll('[data-protected="true"]')
  expect(surfaces.length).toBe(4)
  expect(notices.text()).toContain('Confirm')
  expect(notices.text()).toContain('Cancel')
})

test('review banner names pending plugins and provides a review action', () => {
  const notices = mount(Notices, {
    props: {
      banners: { ...banners, reviewNames: ['notes', 'clock'] },
      notice: null,
      archived: false,
    },
  })
  expect(notices.text()).toContain('notes, clock')
  expect(notices.text()).toContain('Review')
})

test('an oversized header slot is contained above the protected notices', async () => {
  const { default: ChannelHeader } = await import('./ChannelHeader.vue')
  const header = mount(ChannelHeader, {
    props: { channel: 'general', memberListVisible: true },
    slots: { default: '<div style="height: 5000px">Plugin</div>' },
  })
  const slot = header.get('[data-testid="header-slot"]')
  expect(slot.classes()).toContain('max-h-16')
  expect(slot.classes()).toContain('overflow-y-auto')
})

test('approval buttons use the protected border for focus', () => {
  const card = mount(ApprovalCard, {
    props: {
      channelId: 'channel',
      prompt: { approvalId: 'a', tool: 'shell', summary: 'run', view: 'pending' },
    },
  })
  for (const button of card.findAll('button'))
    expect(button.classes()).toContain('focus-visible:ring-[var(--protected-border)]')
})

test('approval decisions in a live transcript include the originating channel', async () => {
  const { default: MessageList } = await import('./MessageList.vue')
  const { decideApproval } = await import('../bridge/api')
  const list = mount(MessageList, {
    props: {
      conversationId: 'second-channel',
      messages: [],
      live: [{ type: 'approval', approval_id: 'approval-2', tool: 'shell', summary: 'Run tests' }],
      running: true,
      newAnchorId: null,
      prompts: [{ approvalId: 'approval-2', tool: 'shell', summary: 'Run tests', view: 'pending' }],
    },
  })
  const approve = list.findAll('button').find((button) => button.text() === 'Approve')!
  await approve.trigger('click')
  expect(decideApproval).toHaveBeenCalledWith('second-channel', 'approval-2', 'approve')
})

test('long notices wrap inside their inset alert and preserve the full error', () => {
  const error = 'Codex setup failed: ' + '/a-long-cache-path'.repeat(40)
  const notices = mount(Notices, { props: { banners, notice: error, archived: false } })
  for (const alert of notices.findAll('[role="alert"]')) {
    expect(alert.classes()).toContain('w-auto')
    expect(alert.classes()).not.toContain('w-full')
  }
  const description = notices.get('[data-slot="alert-description"]')
  expect(description.text()).toBe(error)
  expect(description.classes()).toContain('[overflow-wrap:anywhere]')
})
