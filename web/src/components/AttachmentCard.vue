<script setup lang="ts">
import { ref, watch } from 'vue'
import { FileText, File, Image, X } from '@lucide/vue'
import { Button } from './ui/button'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from './ui/dialog'
import { attachmentPreview } from '@/bridge/api'
import type { AttachmentDto } from '@/bridge/types'
const props = defineProps<{ attachment: AttachmentDto; removable?: boolean; disabled?: boolean }>()
const emit = defineEmits<{ remove: [id: string] }>()
const preview = ref('')
const previewError = ref('')
const open = ref(false)
let request = 0
watch(
  () => props.attachment.id,
  async () => {
    const current = ++request
    preview.value = ''
    previewError.value = ''
    if (!props.attachment.mediaType.startsWith('image/')) return
    try {
      const url = await attachmentPreview(props.attachment.id)
      if (current === request) preview.value = url
    } catch {
      if (current === request) previewError.value = 'Preview unavailable'
    }
  },
  { immediate: true },
)
function size(bytes: number) {
  return bytes < 1024
    ? `${bytes} B`
    : bytes < 1024 * 1024
      ? `${Math.ceil(bytes / 1024)} KB`
      : `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}
</script>
<template>
  <article
    data-ui-id="attachment-card"
    class="relative flex w-36 shrink-0 flex-col overflow-hidden rounded-lg border bg-muted/30"
  >
    <button
      type="button"
      class="flex h-20 items-center justify-center bg-muted/40 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
      :aria-label="`Preview ${attachment.name}`"
      :disabled="!preview && attachment.text == null"
      @click="open = true"
    >
      <img v-if="preview" :src="preview" alt="" class="size-full object-cover" />
      <component
        v-else
        :is="
          attachment.mediaType.startsWith('image/')
            ? Image
            : attachment.mediaType === 'application/pdf' || attachment.mediaType === 'text/plain'
              ? FileText
              : File
        "
        class="size-8 text-muted-foreground"
      />
    </button>
    <div class="flex min-w-0 flex-col gap-0.5 px-2 py-1.5">
      <span class="truncate text-xs font-medium" :title="attachment.name">
        {{ attachment.name }}
      </span>
      <span class="text-xs text-muted-foreground">{{ previewError || size(attachment.size) }}</span>
    </div>
    <Button
      v-if="removable"
      type="button"
      size="icon-xs"
      variant="secondary"
      class="absolute right-1 top-1"
      :disabled="disabled"
      :aria-label="`Remove ${attachment.name}`"
      @click="emit('remove', attachment.id)"
    >
      <X />
    </Button>
    <Dialog v-model:open="open">
      <DialogContent class="max-h-[85vh] overflow-auto sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle class="break-all">{{ attachment.name }}</DialogTitle>
          <DialogDescription>
            {{ size(attachment.size) }}
            <template v-if="attachment.mediaType === 'application/pdf'">
              · Extracted PDF text
            </template>
          </DialogDescription>
        </DialogHeader>
        <img
          v-if="preview"
          :src="preview"
          :alt="attachment.name"
          class="max-h-[65vh] w-full object-contain"
        />
        <pre v-else class="whitespace-pre-wrap break-words text-sm">{{ attachment.text }}</pre>
      </DialogContent>
    </Dialog>
  </article>
</template>
