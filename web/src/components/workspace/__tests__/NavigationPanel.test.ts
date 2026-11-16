import { expect, test } from 'vite-plus/test'
import { shallowMount } from '@vue/test-utils'
import fixture from '@/bridge/fixtures/snapshot.json'
import type { SnapshotDto } from '@/bridge/types'
import ChannelList from '../../ChannelList.vue'
import ServerRail from '../../ServerRail.vue'
import UserBar from '../UserBar.vue'
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

test('the panel mounts the project rail by default', () => {
  expect(mountPanel('p').findComponent(ServerRail).exists()).toBe(true)
})

test('rail false leaves out the project rail and keeps the channel list', () => {
  const wrapper = shallowMount(NavigationPanel, {
    props: { projects, snapshot, page: 'conversation', rail: false },
  })
  expect(wrapper.findComponent(ServerRail).exists()).toBe(false)
  expect(wrapper.findComponent(ChannelList).exists()).toBe(true)
})

test('rail false in the compact layout leaves the user bar only', () => {
  const wrapper = shallowMount(NavigationPanel, {
    props: { projects, snapshot, page: 'conversation', rail: false, compact: true },
  })
  expect(wrapper.findComponent(ServerRail).exists()).toBe(false)
  expect(wrapper.findComponent(ChannelList).exists()).toBe(false)
  expect(wrapper.findComponent(UserBar).exists()).toBe(true)
})
