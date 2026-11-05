<script setup lang="ts">
import type {
  SplitterResizeHandleEmits,
  SplitterResizeHandleProps,
} from 'reka-ui';
import type { HTMLAttributes } from 'vue';
import { reactiveOmit } from '@vueuse/core';
import { cn } from '@/lib/utils';
import { EllipsisVertical } from '@lucide/vue';
import { SplitterResizeHandle, useForwardPropsEmits } from 'reka-ui';

const props = defineProps<
  SplitterResizeHandleProps & {
    class?: HTMLAttributes['class'];
    withHandle?: boolean;
  }
>();
const emits = defineEmits<SplitterResizeHandleEmits>();

const delegatedProps = reactiveOmit(props, 'class', 'withHandle');
const forwarded = useForwardPropsEmits(delegatedProps, emits);
</script>

<template>
  <SplitterResizeHandle
    data-slot="resizable-handle"
    v-bind="forwarded"
    :class="
      cn(
        'relative flex w-1.5 shrink-0 items-center justify-center rounded-sm bg-transparent text-muted-foreground transition-colors hover:bg-primary/10 hover:text-primary focus-visible:text-primary focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none data-[orientation=vertical]:h-1.5 data-[orientation=vertical]:w-full [&[data-orientation=vertical]>svg]:rotate-90',
        props.class
      )
    "
  >
    <template v-if="props.withHandle">
      <slot>
        <EllipsisVertical
          aria-hidden="true"
          class="pointer-events-none h-5 w-3 shrink-0"
        />
      </slot>
    </template>
  </SplitterResizeHandle>
</template>
