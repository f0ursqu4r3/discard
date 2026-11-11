import { expect, test, vi } from 'vite-plus/test'
import { invoke } from '@tauri-apps/api/core'
import {
  getChannelDetails,
  getProjectChannels,
  getProjectMembers,
  selectCanonicalBranch,
  selectCanonicalProjectChannel,
  setProjectMainAgent,
  updateChannelTopic,
} from '../api'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

test('project channel APIs use exact project and channel IDs', async () => {
  await getProjectChannels('project-1')
  await getProjectMembers('project-1')
  await getChannelDetails('project-1', 'channel-1')
  await updateChannelTopic('project-1', 'channel-1', 4, 'new topic')
  await setProjectMainAgent('project-1', 'agent-1')
  await selectCanonicalProjectChannel('project-1', 'channel-1')
  await selectCanonicalBranch('project-1', 'feature/x', 'channel-1')

  expect(invoke).toHaveBeenNthCalledWith(1, 'get_project_channels', { projectId: 'project-1' })
  expect(invoke).toHaveBeenNthCalledWith(2, 'get_project_members', { projectId: 'project-1' })
  expect(invoke).toHaveBeenNthCalledWith(3, 'get_channel_details', {
    projectId: 'project-1',
    channelId: 'channel-1',
  })
  expect(invoke).toHaveBeenNthCalledWith(4, 'update_channel_topic', {
    projectId: 'project-1',
    channelId: 'channel-1',
    expectedVersion: 4,
    text: 'new topic',
  })
  expect(invoke).toHaveBeenNthCalledWith(5, 'set_project_main_agent', {
    projectId: 'project-1',
    agentId: 'agent-1',
  })
  expect(invoke).toHaveBeenNthCalledWith(6, 'select_canonical_project_channel', {
    projectId: 'project-1',
    channelId: 'channel-1',
  })
  expect(invoke).toHaveBeenNthCalledWith(7, 'select_canonical_branch', {
    projectId: 'project-1',
    branch: 'feature/x',
    channelId: 'channel-1',
  })
})
