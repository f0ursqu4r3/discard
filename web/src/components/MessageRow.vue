<script setup lang="ts">
import type { MessageDto, PromptDto } from '../bridge/types';
import { Avatar, AvatarFallback } from './ui/avatar';
import { Marker, MarkerContent, MarkerIcon } from './ui/marker';
import { Info, Copy, Quote, MessagesSquare } from '@lucide/vue';
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from './ui/tooltip';
import AgentAvatar from './agents/AgentAvatar.vue';
import { Button } from './ui/button';
import BlockView from './BlockView.vue';
import ContextMenuHost from './ContextMenuHost.vue';
import { runAction } from '../bridge/api';
import { computed } from 'vue';
import { useAppearance } from '@/bridge/appearance';
import { serverInitials } from './layout';
import { messageTime } from './messages';
const props = defineProps<{
  channelId: string;
  message: MessageDto;
  agent?: string;
  agentAvatar?: string | null;
  prompts?: PromptDto[];
  showCursor?: boolean;
  preview?: boolean;
  quoteDisabled?: boolean;
}>();
const emit = defineEmits<{ quote: [message: MessageDto] }>();
function quote() {
  if (target.value?.hasText && !props.quoteDisabled)
    emit('quote', props.message);
}
const date = computed(() => messageTime(props.message));
const timeLabel = computed(() =>
  date.value?.toLocaleTimeString(undefined, {
    hour: 'numeric',
    minute: '2-digit',
  })
);
const appearance = useAppearance();
const compact = computed(() => appearance.value.messageDensity === 'compact');
// Runtime system notes are plain text. Keep other block types on the full renderer.
const systemNote = computed(
  () =>
    props.message.role === 'system' &&
    props.message.blocks.every((block) => block.type === 'markdown')
);
const systemText = computed(() =>
  props.message.blocks
    .flatMap((block) => (block.type === 'markdown' ? [block.text] : []))
    .join('\n')
);
const authorName = computed(
  () => props.message.author?.name ?? props.agent ?? 'Agent'
);
const authorAvatar = computed(() =>
  props.message.author ? props.message.author.avatar : props.agentAvatar
);
const target = computed(() =>
  props.preview || props.message.id === 'live'
    ? null
    : {
        type: 'message' as const,
        id: props.message.id,
        hasText: props.message.blocks.some(
          (block) => 'text' in block && !!block.text
        ),
      }
);
async function copy() {
  if (!target.value) return;
  const text = await runAction('copy_message_text', target.value);
  if (text !== null) await navigator.clipboard.writeText(text);
}
</script>
<template>
  <ContextMenuHost
    :target="target"
    :quote-enabled="!!target?.hasText && !quoteDisabled"
    @quote="quote"
  >
    <Marker
      v-if="systemNote"
      as="article"
      :data-ui-id="`message-${message.id}`"
      aria-label="System note"
      class="px-3 sm:px-6"
      :class="compact ? 'py-1' : 'py-2'"
    >
      <MarkerIcon><Info /></MarkerIcon>
      <MarkerContent class="whitespace-pre-wrap">{{
        systemText
      }}</MarkerContent>
    </Marker>
    <article
      v-else
      :data-ui-id="`message-${message.id}`"
      :tabindex="target ? 0 : undefined"
      class="group relative flex gap-2 px-3 text-sm leading-relaxed hover:bg-muted/30 sm:px-6"
      :class="compact ? 'py-1' : 'py-2'"
    >
      <div
        data-testid="message-time-gutter"
        class="w-[8ch] shrink-0 text-right text-xs leading-[1.625rem] tabular-nums text-muted-foreground"
      >
        <time
          v-if="date && !message.continuesGroup"
          data-testid="message-timestamp"
          :datetime="date.toISOString()"
          :title="date.toLocaleString()"
          class="whitespace-nowrap"
        >
          {{ timeLabel }}
        </time>
      </div>
      <div
        v-if="appearance.showMessageAvatars"
        data-testid="message-avatar"
        class="w-5 shrink-0 pt-0.5"
      >
        <AgentAvatar
          v-if="!message.continuesGroup && message.role === 'agent'"
          :name="authorName"
          :avatar="authorAvatar"
          class="size-5"
        />
        <Avatar v-else-if="!message.continuesGroup" class="size-5">
          <AvatarFallback class="bg-primary/20 text-primary">
            {{
              serverInitials(
                message.role === 'user' ? 'You' : agent || message.role
              )
            }}
          </AvatarFallback>
        </Avatar>
      </div>
      <div class="min-w-0 flex-1">
        <strong data-testid="message-identity" class="mr-2 font-semibold">
          {{
            message.role === 'user'
              ? 'You'
              : message.role === 'agent'
                ? authorName
                : 'System'
          }}
        </strong>
        <BlockView
          v-for="(block, i) in message.blocks"
          :key="i"
          :channel-id="channelId"
          :block="block"
          :inline="i === 0"
          :prompts="prompts"
          :message-id="target?.id"
          :index="i"
        />
        <span
          v-if="showCursor"
          data-testid="stream-cursor"
          class="text-muted-foreground"
          >▍</span
        >
      </div>
      <TooltipProvider
        v-if="target"
        :delay-duration="500"
        :skip-delay-duration="0"
      >
        <div
          role="group"
          aria-label="Message actions"
          class="absolute right-0 top-[-80%] z-10 invisible flex gap-0.5 rounded-md border bg-background p-1 shadow-sm group-hover:visible group-focus-within:visible"
        >
          <Tooltip>
            <TooltipTrigger as-child>
              <Button
                :disabled="!target.hasText"
                variant="ghost"
                size="icon-xs"
                aria-label="Copy message"
                @click="copy"
              >
                <Copy aria-hidden="true" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Copy message</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger as-child>
              <Button
                :disabled="quoteDisabled || !target.hasText"
                variant="ghost"
                size="icon-xs"
                aria-label="Quote message"
                @click="quote"
              >
                <Quote aria-hidden="true" />
              </Button>
            </TooltipTrigger>
            <TooltipContent>Quote message</TooltipContent>
          </Tooltip>
          <Tooltip>
            <TooltipTrigger as-child>
              <span
                class="inline-flex"
                tabindex="0"
                aria-label="Start thread — Soon"
                aria-disabled="true"
              >
                <!-- TODO: Open a clarification thread with channel history through this source message. -->
                <Button
                  disabled
                  variant="ghost"
                  size="icon-xs"
                  aria-label="Start thread — Soon"
                >
                  <MessagesSquare aria-hidden="true" />
                </Button>
              </span>
            </TooltipTrigger>
            <TooltipContent>Start thread · Soon</TooltipContent>
          </Tooltip>
        </div>
      </TooltipProvider>
    </article>
  </ContextMenuHost>
</template>
