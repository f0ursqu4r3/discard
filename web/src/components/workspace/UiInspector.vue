<script setup lang="ts">
import { onMounted, onUnmounted, shallowRef, ref } from 'vue'
import { Copy, X } from '@lucide/vue'
import { toast } from 'vue-sonner'
import { Button } from '../ui/button'
const emit = defineEmits<{ close: [] }>()
const selected = shallowRef<HTMLElement | null>(null)
const rect = shallowRef<DOMRect | null>(null)
const name = ref('Move over a panel or control')
function track(event: Event) {
  if (!(event.target instanceof HTMLElement)) return
  if (event.target.closest('[data-ui-inspector]')) return
  const element = event.target.closest<HTMLElement>(
    'button, input, textarea, [data-ui-id], [data-slot]',
  )
  if (!element) return
  selected.value = element
  name.value =
    element.dataset.uiId ||
    element.id ||
    `${element.closest('[data-ui-id]')?.getAttribute('data-ui-id') || 'ui'}/${element.getAttribute('aria-label') || element.dataset.slot || element.tagName.toLowerCase()}`
  measure()
}
function measure() {
  rect.value = selected.value?.isConnected ? selected.value.getBoundingClientRect() : null
}
function key(event: KeyboardEvent) {
  if (event.key === 'Escape') emit('close')
}
async function copy() {
  try {
    await navigator.clipboard.writeText(name.value)
    toast.success('UI ID copied')
  } catch {
    toast.error('Could not copy UI ID')
  }
}
onMounted(() => {
  document.addEventListener('pointerover', track)
  document.addEventListener('focusin', track)
  document.addEventListener('scroll', measure, true)
  window.addEventListener('resize', measure)
  window.addEventListener('keydown', key)
})
onUnmounted(() => {
  document.removeEventListener('pointerover', track)
  document.removeEventListener('focusin', track)
  document.removeEventListener('scroll', measure, true)
  window.removeEventListener('resize', measure)
  window.removeEventListener('keydown', key)
})
</script>
<template>
  <Teleport to="body">
    <div
      v-if="rect"
      data-ui-inspector
      class="pointer-events-none fixed z-50 rounded-sm border-2 border-primary bg-primary/5"
      :style="{
        left: `${rect.left}px`,
        top: `${rect.top}px`,
        width: `${rect.width}px`,
        height: `${rect.height}px`,
      }"
    />
    <div
      data-ui-inspector
      role="status"
      class="fixed inset-x-3 bottom-3 z-50 mx-auto flex w-fit max-w-full items-center gap-3 rounded-lg border border-border bg-popover px-3 py-2 text-xs text-popover-foreground shadow-lg"
    >
      <span class="truncate font-mono">{{ name }}</span
      ><Button variant="ghost" size="icon-xs" aria-label="Copy UI ID" @click="copy"><Copy /></Button
      ><Button variant="ghost" size="icon-xs" aria-label="Close UI inspector" @click="emit('close')"
        ><X
      /></Button>
    </div>
  </Teleport>
</template>
