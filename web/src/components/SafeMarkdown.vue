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
  targets.value = rendered.value.blocks.flatMap(block => {
    const element = root.querySelector(`[data-markdown-code="${block.index}"]`)
    return element ? [{ element, block }] : []
  })
}
watch(rendered, bindBlocks)
onMounted(bindBlocks)
</script>
<template>
  <div><SafeHtml ref="host" class="break-words" :html="rendered.html" />
  <Teleport v-for="target in targets" :key="target.block.index" :to="target.element"><CodeBlock :text="target.block.text" :language="target.block.language" copy-text /></Teleport></div>
</template>
