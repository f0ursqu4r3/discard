import type { ReviewStateDto } from '../../bridge/types'

const LABELS: Partial<Record<ReviewStateDto, string>> = {
  approved: 'Approved',
  changes_requested: 'Changes requested',
  changes_requested_new_commits: 'Changes requested',
  approval_out_of_date: 'Out of date',
  merged: 'Merged',
}

/** The tab badge text for a review state, or null when no badge shows. */
export function reviewBadge(state: ReviewStateDto | null): string | null {
  return (state && LABELS[state]) ?? null
}
