<script setup lang="ts">
import type { PrimitiveProps } from 'reka-ui'

import type { HTMLAttributes } from 'vue'
import type { ButtonVariants } from '@/components/ui/button'
import { useClipboard } from '@vueuse/core'
import { cn } from 'cn'
import { Button } from '@/components/ui/button'

defineOptions({ inheritAttrs: false })

const props = withDefaults(
  defineProps<
    PrimitiveProps & {
      value: string
      variant?: ButtonVariants['variant']
      size?: ButtonVariants['size']
      class?: HTMLAttributes['class']
    }
  >(),
  {
    as: 'button',
    variant: 'ghost',
    size: 'xs',
  },
)

const { copy, copied } = useClipboard({ copiedDuring: 2000, legacy: true })
</script>

<template>
  <Button
    v-bind="$attrs"
    data-slot="code-block-copy-button"
    :data-copied="copied || undefined"
    :as="as"
    :as-child="asChild"
    :variant="variant"
    :size="size"
    :aria-label="copied ? 'Copied' : 'Copy'"
    :class="cn('font-mono text-muted-foreground', props.class)"
    @click="copy(props.value)"
  >
    {{ copied ? 'copied' : 'copy' }}
  </Button>
  <span aria-live="polite" class="sr-only">{{ copied ? 'Copied' : '' }}</span>
</template>
