<script setup lang="ts">
import { computed, nextTick, ref, useId, watch } from 'vue'
import { ChevronLeft, ChevronRight, ChevronsUpDown } from '@lucide/vue'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Field, FieldLabel, FieldSet, FieldLegend } from '@/components/ui/field'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import AgentAvatar from './AgentAvatar.vue'
import ProjectAvatar from '../workspace/ProjectAvatar.vue'
import {
  defaultIconColor,
  formatAgentIcon,
  iconComponent,
  iconLabel,
  parseAgentIcon,
  searchIcons,
  validIconColor,
} from './agent-icon'

const avatar = defineModel<string | null>({ required: true })
withDefaults(defineProps<{ name: string; disabled?: boolean; entity?: 'Agent' | 'Project' }>(), {
  entity: 'Agent',
})
const appearance = computed(() => parseAgentIcon(avatar.value))
const color = computed(() => appearance.value?.color ?? defaultIconColor)
const open = ref(false),
  query = ref(''),
  page = ref(0)
const id = useId()
const results = computed(() => searchIcons(query.value))
const pageSize = 48
const pages = computed(() => Math.max(1, Math.ceil(results.value.length / pageSize)))
const visible = computed(() =>
  results.value.slice(page.value * pageSize, (page.value + 1) * pageSize),
)
const presets = [
  ['Blue', '#789cf5'],
  ['Violet', '#a78bfa'],
  ['Pink', '#f472b6'],
  ['Red', '#ef4444'],
  ['Orange', '#fb923c'],
  ['Yellow', '#facc15'],
  ['Green', '#4ade80'],
  ['Teal', '#2dd4bf'],
] as const
watch(query, () => {
  page.value = 0
})
watch(open, async (value) => {
  if (value) {
    await nextTick()
    document.getElementById(`${id}-search`)?.focus()
  }
})
async function selectIcon(icon: string) {
  avatar.value = formatAgentIcon(icon, color.value)
  open.value = false
  await nextTick()
  document.getElementById(`${id}-trigger`)?.focus()
}
function selectColor(value: string | number) {
  if (validIconColor(String(value)))
    avatar.value = formatAgentIcon(appearance.value?.icon ?? null, String(value))
}
</script>
<template>
  <FieldSet data-ui-id="agent-icon-picker" class="min-w-0 w-full" :disabled="disabled">
    <FieldLegend>{{ entity }} icon</FieldLegend>
    <Popover v-model:open="open">
      <div class="flex min-w-0 items-center gap-4">
        <component
          :is="entity === 'Project' ? ProjectAvatar : AgentAvatar"
          :name="name"
          :avatar="avatar"
          class="size-14"
        />
        <PopoverTrigger as-child>
          <Button
            :id="`${id}-trigger`"
            type="button"
            variant="outline"
            :disabled="disabled"
            class="min-w-0 flex-1 justify-between"
            :aria-label="`Choose ${entity.toLowerCase()} icon`"
          >
            <span class="truncate">
              {{ appearance?.icon ? iconLabel(appearance.icon) : 'Choose an icon' }}
            </span>
            <ChevronsUpDown data-icon="inline-end" />
          </Button>
        </PopoverTrigger>
      </div>
      <PopoverContent
        align="start"
        :collision-padding="16"
        aria-label="Lucide icon picker"
        class="w-88 max-w-[calc(100vw-2rem)] max-h-[min(28rem,var(--reka-popover-content-available-height))] gap-3 overflow-y-auto"
      >
        <Field>
          <FieldLabel :for="`${id}-search`">Search Lucide icons</FieldLabel>
          <Input
            :id="`${id}-search`"
            v-model="query"
            :disabled="disabled"
            placeholder="Search all icons…"
            @keydown.enter.prevent
            @keydown.esc.stop.prevent="open = false"
          />
        </Field>
        <div class="grid grid-cols-6 gap-1 sm:grid-cols-8" aria-label="Lucide icons">
          <Button
            v-for="icon in visible"
            :key="icon"
            type="button"
            :variant="appearance?.icon === icon ? 'secondary' : 'ghost'"
            size="icon-sm"
            :disabled="disabled"
            :title="iconLabel(icon)"
            :aria-label="`Choose ${iconLabel(icon)}`"
            :aria-pressed="appearance?.icon === icon"
            @click="selectIcon(icon)"
          >
            <component :is="iconComponent(icon)" />
          </Button>
        </div>
        <p v-if="!results.length" role="status" class="text-sm text-muted-foreground">
          No matching icons.
        </p>
        <div class="flex items-center justify-between gap-2">
          <span role="status" class="text-xs text-muted-foreground">
            {{ results.length }} icons · {{ page + 1 }} / {{ pages }}
          </span>
          <div class="flex gap-1">
            <Button
              type="button"
              variant="outline"
              size="icon-sm"
              aria-label="Previous icons"
              :disabled="disabled || page === 0"
              @click="page--"
            >
              <ChevronLeft />
            </Button>
            <Button
              type="button"
              variant="outline"
              size="icon-sm"
              aria-label="Next icons"
              :disabled="disabled || page + 1 >= pages"
              @click="page++"
            >
              <ChevronRight />
            </Button>
          </div>
        </div>
      </PopoverContent>
    </Popover>
    <Field>
      <FieldLabel :for="`${id}-color`">{{ entity }} color</FieldLabel>
      <div class="flex flex-wrap items-center gap-2">
        <Button
          v-for="[label, value] in presets"
          :key="value"
          type="button"
          variant="outline"
          size="icon-sm"
          :aria-label="`Use ${label.toLowerCase()}`"
          :aria-pressed="color === value"
          :disabled="disabled"
          @click="selectColor(value)"
        >
          <span class="size-4 rounded-full" :style="{ backgroundColor: value }" />
        </Button>
        <Input
          :id="`${id}-color`"
          type="color"
          :model-value="color"
          :disabled="disabled"
          :aria-label="`Custom ${entity.toLowerCase()} color`"
          class="w-12 p-1"
          @update:model-value="selectColor"
        />
      </div>
    </Field>
  </FieldSet>
</template>
