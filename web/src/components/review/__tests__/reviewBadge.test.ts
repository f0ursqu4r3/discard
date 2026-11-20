import { expect, test } from 'vite-plus/test'
import { reviewBadge } from '../reviewBadge'

test.each([
  ['approved', 'Approved'],
  ['changes_requested', 'Changes requested'],
  ['changes_requested_new_commits', 'Changes requested'],
  ['approval_out_of_date', 'Out of date'],
  ['merged', 'Merged'],
] as const)('%s shows %s', (state, label) => {
  expect(reviewBadge(state)).toBe(label)
})

test.each(['not_reviewed', 'nothing_to_review', null] as const)('%s shows no badge', (state) => {
  expect(reviewBadge(state)).toBeNull()
})
