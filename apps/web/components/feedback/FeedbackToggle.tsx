'use client';

import { useState } from 'react';
import { FeedbackMenu } from './FeedbackMenu';

/**
 * Feedback on a finding card. The menu (and its feedback query) mounts on first open, so a
 * long findings list does not issue one request per finding.
 */
export function FeedbackToggle({ findingId }: { findingId: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div>
      <button
        type="button"
        className="text-xs text-muted-foreground hover:text-foreground hover:underline"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        {open ? 'Hide feedback' : 'Give feedback'}
      </button>
      {open && <FeedbackMenu findingId={findingId} compact />}
    </div>
  );
}
