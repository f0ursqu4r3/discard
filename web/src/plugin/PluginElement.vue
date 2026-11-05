<script setup lang="ts">
import { computed } from 'vue'
import type { ElementDto } from '../bridge/types'
import { pluginAction } from '../bridge/api'
import { Button } from '../components/ui/button'
import { Avatar, AvatarFallback } from '../components/ui/avatar'
import { Badge } from '../components/ui/badge'
import { Separator } from '../components/ui/separator'
import SafeMarkdown from '../components/SafeMarkdown.vue'
import { serverInitials } from '../components/layout'
import { PROP_WHITELIST, clampNumber, colorToken } from './props'
const props = defineProps<{ element: ElementDto; plugin: string }>()
const clean = computed(() =>
  Object.fromEntries(
    Object.entries(props.element.props).filter(([key]) => PROP_WHITELIST.has(key)),
  ),
)
function textOf(element: ElementDto): string {
  return element.children
    .map((child) => ('Text' in child ? child.Text : textOf(child.Element)))
    .join('')
}
const text = computed(() => textOf(props.element))
const style = computed(() => {
  const p = clean.value
  const result: Record<string, string> = {}
  const size = (value: unknown, name: string) =>
    typeof value === 'number' ? `calc(var(--spacing) * ${clampNumber(name, value) / 4})` : undefined
  if (props.element.tag === 'row' || props.element.tag === 'column') {
    for (const [key, css] of [
      ['gap', 'gap'],
      ['padding', 'padding'],
      ['padding_x', 'paddingInline'],
      ['padding_y', 'paddingBlock'],
      ['width', 'width'],
    ] as const) {
      const value = size(p[key], key)
      if (value) result[css] = value
    }
    const bg = colorToken(p.bg)
    if (bg) result.backgroundColor = bg
  }
  const color = colorToken(p.color)
  if (color) result[props.element.tag === 'dot' ? 'backgroundColor' : 'color'] = color
  return Object.keys(result).length ? result : undefined
})
const classes = computed(() => {
  const p = clean.value
  return [
    p.grow === true && 'flex-1',
    p.height_full === true && 'h-full',
    p.width !== undefined && 'shrink-0',
    p.align === 'center' && 'items-center',
    p.align === 'end' && 'items-end',
    p.align === 'start' && 'items-start',
    p.border_bottom === true && 'border-b border-border',
    p.border_top === true && 'border-t border-border',
    p.border_left === true && 'border-l border-border',
    p.border_right === true && 'border-r border-border',
    p.size === 'xs' && 'text-xs',
    p.size === 'sm' && 'text-sm',
    p.size === 'lg' && 'text-lg',
    p.weight === 'bold' && 'font-bold',
    p.weight === 'semibold' && 'font-semibold',
  ]
})
function action() {
  if (typeof clean.value.on_click === 'string')
    void pluginAction(props.plugin, clean.value.on_click)
}
</script>
<template>
  <div
    v-if="element.tag === 'row' || element.tag === 'column'"
    :data-plugin-tag="element.tag"
    class="flex min-w-0"
    :class="[element.tag === 'column' ? 'flex-col' : 'flex-row', classes]"
    :style="style"
  >
    <template v-for="(child, i) in element.children" :key="i">
      <span v-if="'Text' in child">{{ child.Text }}</span>
      <PluginElement v-else :element="child.Element" :plugin="plugin" />
    </template>
  </div>
  <div v-else-if="element.tag === 'text'" data-plugin-tag="text" :class="classes" :style="style">
    {{ text }}
  </div>
  <SafeMarkdown v-else-if="element.tag === 'markdown'" data-plugin-tag="markdown" :text="text" />
  <Button
    v-else-if="element.tag === 'button'"
    data-plugin-tag="button"
    size="sm"
    :variant="
      clean.variant === 'primary'
        ? 'default'
        : clean.variant === 'danger'
          ? 'destructive'
          : clean.variant === 'outline'
            ? 'outline'
            : 'secondary'
    "
    @click="action"
  >
    {{ text }}
  </Button>
  <div v-else-if="element.tag === 'avatar'" data-plugin-tag="avatar" class="relative inline-flex">
    <Avatar :class="clean.size === 'small' ? 'size-6' : clean.size === 'large' ? 'size-10' : ''">
      <AvatarFallback>
        {{ serverInitials(typeof clean.name === 'string' ? clean.name : '?') }}
      </AvatarFallback>
    </Avatar>
    <span
      v-if="colorToken(clean.status)"
      class="absolute -bottom-1 -right-1 size-2 rounded-full border-2 border-sidebar"
      :style="{ backgroundColor: colorToken(clean.status) }"
    />
  </div>
  <Badge v-else-if="element.tag === 'badge'" data-plugin-tag="badge">
    {{ typeof clean.count === 'number' ? Math.floor(clampNumber('count', clean.count)) : 0 }}
  </Badge>
  <Separator v-else-if="element.tag === 'divider'" data-plugin-tag="divider" />
  <div v-else-if="element.tag === 'spacer'" data-plugin-tag="spacer" class="flex-1" />
  <div
    v-else-if="element.tag === 'dot'"
    data-plugin-tag="dot"
    class="size-2 rounded-full bg-muted-foreground"
    :style="style"
  />
</template>
