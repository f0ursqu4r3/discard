<script setup lang="ts">
import { computed } from 'vue'
import type { BlockDto } from '../bridge/types'
import SafeMarkdown from './SafeMarkdown.vue'
import AttachmentCard from './AttachmentCard.vue'
import CodeBlock from './CodeBlock.vue'
import DiffBlock from './DiffBlock.vue'
import ToolCallBlock from './ToolCallBlock.vue'
import ApprovalCard from './ApprovalCard.vue'
import ContextMenuHost from './ContextMenuHost.vue'
import type { ContextTargetDto } from '../bridge/types'
import type { PromptDto } from '../bridge/types'
const props = defineProps<{
  channelId: string
  block: BlockDto
  prompts?: PromptDto[]
  messageId?: string
  index?: number
  inline?: boolean
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
const approval = computed(() => {
  const block = props.block
  if (block.type !== 'approval') return null
  return (
    props.prompts?.find((p) => p.approvalId === block.approval_id) ?? {
      approvalId: block.approval_id,
      tool: block.tool,
      summary: block.summary,
      view: 'answered',
    }
  )
})
</script>
<template>
  <ContextMenuHost :target="target">
    <AttachmentCard v-if="block.type === 'attachment'" :attachment="block.attachment" />
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
    />
    <ApprovalCard :channel-id="channelId" v-else-if="approval" :prompt="approval" />
  </ContextMenuHost>
</template>
