import { expect, test } from 'vite-plus/test'
import { shallowMount } from '@vue/test-utils'
import fixture from '@/bridge/fixtures/snapshot.json'
import type { SnapshotDto } from '@/bridge/types'
import ChannelList from '../../ChannelList.vue'
import NavigationPanel from '../NavigationPanel.vue'

const snapshot = { ...(fixture as SnapshotDto), projectId: 'p' }
const projects = [
  { id: 'p', name: 'One', path: '/one', showArchived: true },
  { id: 'q', name: 'Two', path: '/two' },
]
const mountPanel = (id: string) =>
  shallowMount(NavigationPanel, {
    props: { projects, snapshot: { ...snapshot, projectId: id }, page: 'conversation' },
  })

test('the channel list shows Archived when the open project turns it on', () => {
  expect(mountPanel('p').findComponent(ChannelList).props('showArchived')).toBe(true)
})

test('the channel list hides Archived when the project setting is off or missing', () => {
  expect(mountPanel('q').findComponent(ChannelList).props('showArchived')).toBe(false)
  expect(mountPanel('other').findComponent(ChannelList).props('showArchived')).toBe(false)
})
