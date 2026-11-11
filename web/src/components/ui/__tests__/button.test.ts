/// <reference types="node" />
import { mount } from '@vue/test-utils'
import { expect, test } from 'vite-plus/test'
import Button from '../button/Button.vue'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const tokens = readFileSync(resolve(process.cwd(), 'src/styles/tokens.css'), 'utf8')

test('the destructive button uses the destructive color', () => {
  expect(mount(Button, { props: { variant: 'destructive' } }).classes()).toContain('bg-destructive')
})

test('theme mappings provide shadcn utility variables', () => {
  for (const name of [
    '--color-popover',
    '--color-input',
    '--color-ring',
    '--color-destructive',
    '--radius-lg',
  ]) {
    expect(tokens).toMatch(new RegExp(`${name}\\s*:`))
  }
})
