<script setup lang="ts">
import { ref, watch } from 'vue'
import type { BundledLanguage } from 'shiki'
import { Button } from './ui/button'
import type { ContextTargetDto } from '../bridge/types'
import { runAction } from '../bridge/api'
const props = defineProps<{ text: string; language?: string | null; target?: ContextTargetDto | null; copyText?: boolean }>()
const tokens = ref<{ content: string; color: string }[][]>([])
let generation = 0
watch(() => [props.text, props.language], (_value, _previous, onCleanup) => {
  const current = ++generation
  tokens.value = []
  if (!props.language) return
  const language = props.language as BundledLanguage
  const text = props.text
  const timer = setTimeout(async () => {
    try {
      const { getSingletonHighlighter, createJavaScriptRegexEngine } = await import('shiki')
      const highlighter = await getSingletonHighlighter({ engine: createJavaScriptRegexEngine(), langs: [language], themes: ['github-dark'] })
      await highlighter.loadLanguage(language)
      const result = await highlighter.codeToTokens(text, { lang: language, theme: 'github-dark', includeExplanation: 'scopeName' })
      if (current !== generation) return
      tokens.value = result.tokens.map(line => line.map(token => {
        const scopes = token.explanation?.flatMap(part => part.scopes.map(scope => scope.scopeName)).join(' ') ?? ''
        const color = /comment/.test(scopes) ? 'text-muted-foreground' : /string/.test(scopes) ? 'text-success' : /constant|numeric/.test(scopes) ? 'text-warning' : /keyword|storage|entity/.test(scopes) ? 'text-primary' : 'text-foreground'
        return { content: token.content, color }
      }))
    } catch { if (current === generation) tokens.value = [] }
  }, 80)
  onCleanup(() => { clearTimeout(timer); generation++ })
}, { immediate: true })
async function copy() { if (props.copyText) { await navigator.clipboard.writeText(props.text); return } if (!props.target) return; const text = await runAction('copy_code', props.target); if (text !== null) await navigator.clipboard.writeText(text) }
</script>
<template>
  <div class="rounded-md bg-muted p-2 text-sm">
    <div class="flex items-center justify-between"><span>{{ language || 'Code' }}</span><Button v-if="target || copyText" variant="ghost" size="sm" @click="copy">Copy</Button></div>
    <pre class="overflow-x-auto whitespace-pre-wrap"><code v-if="tokens.length"><template v-for="(line, i) in tokens" :key="i"><span v-for="(token, j) in line" :key="j" :class="token.color">{{ token.content }}</span>{{ i < tokens.length - 1 ? '\n' : '' }}</template></code><code v-else>{{ text }}</code></pre>
  </div>
</template>
