<script setup lang="ts">
import type { ContextTargetDto } from '../bridge/types'
import {
  CodeBlock,
  CodeBlockActions,
  CodeBlockContent,
  CodeBlockCopyButton,
  CodeBlockHeader,
  CodeBlockLine,
  CodeBlockTitle,
  CodeBlockToken,
  useHighlightedCode,
} from './ui/code-block'

const props = defineProps<{
  text: string
  language?: string | null
  target?: ContextTargetDto | null
  copyText?: boolean
}>()
const lines = useHighlightedCode(
  () => props.text,
  () => props.language ?? 'text',
)
</script>
<template>
  <CodeBlock>
    <CodeBlockHeader>
      <CodeBlockTitle>{{ language || 'Code' }}</CodeBlockTitle>
      <CodeBlockActions v-if="target || copyText">
        <CodeBlockCopyButton :value="text" />
      </CodeBlockActions>
    </CodeBlockHeader>
    <CodeBlockContent
      :wrap="
        ['sh', 'bash', 'zsh', 'shell', 'shellscript', 'powershell', 'ps1'].includes(
          (language || '').toLowerCase(),
        )
      "
    >
      <CodeBlockLine v-for="(line, index) in lines" :key="index">
        <CodeBlockToken v-for="(token, tokenIndex) in line" :key="tokenIndex" :token="token" />
      </CodeBlockLine>
    </CodeBlockContent>
  </CodeBlock>
</template>
