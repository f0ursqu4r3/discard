<script setup lang="ts">
import { ChevronDown, ChevronRight } from '@lucide/vue'
import { computed, ref } from 'vue'
import type { ChangeFileDto } from '../../bridge/types'
import { Input } from '../ui/input'
import { buildTree, filterFiles, flattenTree, statusTone } from './tree'

const props = defineProps<{ files: ChangeFileDto[]; selected: string | null }>()
const emit = defineEmits<{ select: [path: string] }>()

const filter = ref('')
const collapsed = ref(new Set<string>())
const filtering = computed(() => filter.value.trim() !== '')
const matches = computed(() => filterFiles(props.files, filter.value))
// A filter opens every folder, so a match is never hidden in a closed folder.
const rows = computed(() =>
  flattenTree(buildTree(matches.value), filtering.value ? new Set() : collapsed.value),
)

function toggle(path: string) {
  const next = new Set(collapsed.value)
  if (!next.delete(path)) next.add(path)
  collapsed.value = next
}
const indent = (depth: number) => ({ paddingLeft: `calc(0.5rem + ${depth} * 0.75rem)` })
</script>

<template>
  <div data-ui-id="changes-tree" class="flex h-full min-h-0 flex-col">
    <div class="shrink-0 border-b border-border/60 p-2">
      <Input
        :model-value="filter"
        type="search"
        placeholder="Filter files"
        aria-label="Filter files"
        class="h-7 font-mono text-xs md:text-xs"
        @update:model-value="filter = String($event)"
      />
    </div>
    <ul class="min-h-0 flex-1 overflow-y-auto py-1" aria-label="Changed files">
      <li v-for="row in rows" :key="row.node.path + row.node.type">
        <button
          v-if="row.node.type === 'folder'"
          type="button"
          data-tree-folder
          :aria-expanded="filtering || !collapsed.has(row.node.path)"
          :title="row.node.path"
          class="flex w-full min-w-0 items-center gap-1 py-1 pr-2 text-left font-mono text-xs text-muted-foreground outline-none hover:bg-muted/50 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:ring-inset"
          :style="indent(row.depth)"
          @click="toggle(row.node.path)"
        >
          <component
            :is="filtering || !collapsed.has(row.node.path) ? ChevronDown : ChevronRight"
            aria-hidden="true"
            class="size-3.5 shrink-0"
          />
          <span class="min-w-0 truncate">{{ row.node.name }}</span>
        </button>
        <button
          v-else
          type="button"
          data-tree-file
          :data-path="row.node.path"
          :aria-current="selected === row.node.path || undefined"
          :title="
            row.node.file.oldPath ? `${row.node.file.oldPath} → ${row.node.path}` : row.node.path
          "
          class="flex w-full min-w-0 items-center gap-1.5 py-1 pr-2 text-left font-mono text-xs outline-none hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring/50 focus-visible:ring-inset"
          :class="selected === row.node.path ? 'bg-muted text-foreground' : 'text-foreground/80'"
          :style="indent(row.depth)"
          @click="emit('select', row.node.path)"
        >
          <span
            data-status
            class="w-3.5 shrink-0 text-center font-semibold"
            :class="statusTone(row.node.file.status)"
            :aria-label="`Status ${row.node.file.status}`"
          >
            {{ row.node.file.status }}
          </span>
          <span class="min-w-0 flex-1 truncate">{{ row.node.name }}</span>
          <span v-if="row.node.file.binary" class="shrink-0 text-muted-foreground">binary</span>
          <span v-else class="flex shrink-0 gap-1">
            <span class="text-success">+{{ row.node.file.additions }}</span>
            <span class="text-destructive">−{{ row.node.file.deletions }}</span>
          </span>
        </button>
      </li>
      <li v-if="!rows.length" class="px-3 py-2 text-xs text-muted-foreground">
        No file matches the filter.
      </li>
    </ul>
  </div>
</template>
