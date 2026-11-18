<script setup lang="ts">
import { computed } from 'vue'
import type { BlockDto } from '../bridge/types'
import SafeMarkdown from './SafeMarkdown.vue'
import AttachmentCard from './AttachmentCard.vue'
import CodeBlock from './CodeBlock.vue'
import DiffBlock from './DiffBlock.vue'
import ToolCallBlock from './ToolCallBlock.vue'
import ApprovalMark from './ApprovalMark.vue'
import ChangesCard from './ChangesCard.vue'
import ChannelLinkCard from './ChannelLinkCard.vue'
import ContextMenuHost from './ContextMenuHost.vue'
import type { ContextTargetDto } from '../bridge/types'
import type { PromptDto } from '../bridge/types'
import { approvalStatus, type ApprovalBlock } from './messages'
const props = defineProps<{
  channelId: string
  block: BlockDto
  prompts?: PromptDto[]
  /** The approval block that belongs to this tool call. */
  approvalBlock?: ApprovalBlock
  messageId?: string
  index?: number
  inline?: boolean
  running?: boolean
}>()
const target = computed<ContextTargetDto | null>(() => {
  const block = props.block
  if (!props.messageId || props.index === undefined) return null
  if (block.type === 'code')
    return { type: 'code_block', messageId: props.messageId, block: props.index }
  if (block.type === 'diff')
    return { type: 'diff_block', messageId: props.messageId, block: props.index }
  if (block.type === 'tool_call')
    return {
      type: 'tool_call',
      messageId: props.messageId,
      block: props.index,
      hasOutput: block.output !== null,
    }
  return null
})
const status = computed(() => approvalStatus(props.approvalBlock, props.prompts))
// An approval block without a matching call shows as its own row. Its status comes from its own block.
const standalone = computed(() => {
  const block = props.block
  if (block.type !== 'approval') return null
  return { block, status: approvalStatus(block, props.prompts) }
})
</script>
<template>
  <ContextMenuHost :target="target">
    <ChangesCard v-if="block.type === 'changes'" :block="block" />
    <ChannelLinkCard v-else-if="block.type === 'channel_link'" :block="block" />
    <AttachmentCard v-else-if="block.type === 'attachment'" :attachment="block.attachment" />
    <SafeMarkdown v-else-if="block.type === 'markdown'" :text="block.text" :inline="inline" />
    <DiffBlock
      v-else-if="
        block.type === 'diff' ||
        (block.type === 'code' && ['diff', 'patch'].includes((block.language || '').toLowerCase()))
      "
      :text="block.text"
      :target="target"
    />
    <CodeBlock
      v-else-if="block.type === 'code'"
      :text="block.text"
      :language="block.language"
      :target="target"
    />
    <ToolCallBlock
      v-else-if="block.type === 'tool_call'"
      :name="block.name"
      :input="block.input"
      :output="block.output"
      :is-error="block.is_error"
      :sandboxed="block.sandboxed"
      :running="running"
      :approval="status"
    />
    <div
      v-else-if="standalone"
      data-protected="true"
      class="tool-call my-3 flex min-h-7 min-w-0 items-center gap-2 rounded-none border-l-2 border-border px-2 py-1 font-mono text-xs text-muted-foreground"
    >
      <span aria-hidden="true" class="opacity-60">$</span>
      <span class="min-w-0 truncate text-foreground/80" :title="standalone.block.tool">
        {{ standalone.block.tool }}
      </span>
      <span
        v-if="standalone.block.summary"
        class="min-w-0 flex-1 truncate"
        :title="standalone.block.summary"
      >
        {{ standalone.block.summary }}
      </span>
      <span class="ml-auto shrink-0 pl-2 text-xs">
        <ApprovalMark :status="standalone.status" />
      </span>
    </div>
  </ContextMenuHost>
</template>
