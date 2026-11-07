<script setup lang="ts">
import { computed, nextTick, onMounted, ref, shallowRef, watch } from 'vue'
import SafeHtml from './SafeHtml.vue'
import CodeBlock from './CodeBlock.vue'
import { markdownWithBlocks, type MarkdownCode } from '../lib/markdown'
const props = defineProps<{ text: string }>()
const rendered = computed(() => markdownWithBlocks(props.text))
const host = ref<InstanceType<typeof SafeHtml> | null>(null)
const targets = shallowRef<{ element: Element; block: MarkdownCode }[]>([])
let generation = 0
async function bindBlocks() {
  const current = ++generation
  targets.value = []
  await nextTick()
  if (current !== generation || !host.value) return
  const root = host.value.$el as HTMLElement
  targets.value = rendered.value.blocks.flatMap((block) => {
    const element = root.querySelector(`[data-markdown-code="${block.index}"]`)
    return element ? [{ element, block }] : []
  })
}
watch(rendered, bindBlocks)
onMounted(bindBlocks)
</script>
<template>
  <div>
    <SafeHtml
      ref="host"
      class="break-words [&_a]:text-primary [&_a]:underline [&_blockquote]:border-l-2 [&_blockquote]:border-border [&_blockquote]:pl-3 [&_blockquote]:text-muted-foreground [&_code:not([data-slot=code-block-code])]:rounded-sm [&_code:not([data-slot=code-block-code])]:bg-muted [&_code:not([data-slot=code-block-code])]:px-1 [&_h1]:mb-3 [&_h1]:text-xl [&_h2]:mb-2 [&_h2]:text-lg [&_h3]:font-semibold [&_li]:my-1 [&_ol]:list-decimal [&_ol]:pl-5 [&_p]:mb-3 [&_[data-markdown-code]]:my-3 [&_p:last-child]:mb-0 [&_ul]:list-disc [&_ul]:pl-5"
      :html="rendered.html"
    />
    <Teleport v-for="target in targets" :key="target.block.index" :to="target.element">
      <CodeBlock :text="target.block.text" :language="target.block.language" copy-text />
    </Teleport>
  </div>
</template>
