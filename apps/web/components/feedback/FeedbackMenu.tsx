'use client';

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { useCurrentOrg } from '@/components/org/OrgContext';
import { Button } from '@/components/ui/button';
import { Textarea } from '@/components/ui/input';
import { errorMessage } from '@/components/ui/states';
import { useToast } from '@/components/ui/toast';
import { ApiError } from '@/lib/api-client';
import { submitFeedback } from '@/lib/api/endpoints';
import type { FeedbackInput, FindingFeedback, Verdict } from '@/lib/api/pending';
import { MAX_COMMENT_LENGTH, SUPPRESSIBLE, VERDICTS, applyVerdict } from '@/lib/feedback';
import { feedbackQuery, keys } from '@/lib/queries';
import { canMaintain } from '@/lib/roles';
import { cn } from '@/lib/utils';
import { DEFAULT_SUPPRESS, SuppressOption, type SuppressState } from './SuppressOption';

export const SUPPRESS_ROLE_MESSAGE =
  'Your verdict was saved, but suppressing requires the maintainer role.';

/**
 * Sends the verdict. When the suppression is refused (403) the verdict is sent again without
 * it, so the verdict is saved either way; the caller then shows the role message.
 */
export async function sendFeedback(
  findingId: string,
  input: FeedbackInput,
): Promise<{ feedback: FindingFeedback; suppressionRefused: boolean }> {
  try {
    return { feedback: await submitFeedback(findingId, input), suppressionRefused: false };
  } catch (error) {
    if (input.create_suppression && error instanceof ApiError && error.status === 403) {
      const verdictOnly: FeedbackInput = { verdict: input.verdict };
      if (input.comment !== undefined) verdictOnly.comment = input.comment;
      return { feedback: await submitFeedback(findingId, verdictOnly), suppressionRefused: true };
    }
    throw error;
  }
}

/**
 * Useful · False positive · Already handled · Not relevant · Intentional, with an optional
 * comment. The verdict is applied optimistically and rolled back (with a toast) on failure.
 */
export function FeedbackMenu({
  findingId,
  compact = false,
}: {
  findingId: string;
  compact?: boolean;
}) {
  const org = useCurrentOrg();
  const maintainer = canMaintain(org?.role);
  const queryClient = useQueryClient();
  const toast = useToast();
  const feedback = useQuery(feedbackQuery(findingId));
  const [comment, setComment] = useState('');
  const [showComment, setShowComment] = useState(false);
  const [suppress, setSuppress] = useState<SuppressState>(DEFAULT_SUPPRESS);
  const key = keys.feedback(findingId);

  const mutation = useMutation({
    mutationFn: (input: FeedbackInput) => sendFeedback(findingId, input),
    onMutate: async (input) => {
      await queryClient.cancelQueries({ queryKey: key });
      const previous = queryClient.getQueryData<FindingFeedback>(key);
      queryClient.setQueryData<FindingFeedback>(key, (prev) =>
        applyVerdict(prev, input.verdict, input.comment ?? null),
      );
      return { previous };
    },
    onError: (error, _input, context) => {
      queryClient.setQueryData(key, context?.previous);
      toast.show(`Could not save your feedback: ${errorMessage(error, 'request failed')}`, 'error');
    },
    onSuccess: ({ feedback: saved, suppressionRefused }) => {
      queryClient.setQueryData(key, saved);
      if (suppressionRefused) toast.show(SUPPRESS_ROLE_MESSAGE, 'error');
      setSuppress(DEFAULT_SUPPRESS);
    },
  });

  function choose(verdict: Verdict) {
    const input: FeedbackInput = { verdict };
    const text = comment.trim();
    if (text) input.comment = text.slice(0, MAX_COMMENT_LENGTH);
    if (maintainer && suppress.enabled && SUPPRESSIBLE.has(verdict)) {
      input.create_suppression = {
        kind: suppress.kind,
        reason: suppress.reason.trim() || `marked ${verdict.replace('_', ' ')}`,
      };
    }
    mutation.mutate(input);
  }

  const mine = feedback.data?.mine?.verdict;
  const counts = feedback.data?.counts;

  return (
    <div
      className={cn('space-y-2', !compact && 'rounded-md border p-3')}
      aria-label="Feedback"
      role="group"
    >
      <div className="flex flex-wrap items-center gap-1.5">
        {!compact && <span className="mr-1 text-sm font-medium">Feedback</span>}
        {VERDICTS.map(({ value, label }) => (
          <Button
            key={value}
            type="button"
            size="sm"
            variant={mine === value ? 'default' : 'outline'}
            aria-pressed={mine === value}
            disabled={mutation.isPending}
            onClick={() => choose(value)}
          >
            {label}
            {counts && <span className="tabular-nums opacity-70">{counts[value]}</span>}
          </Button>
        ))}
        <Button
          type="button"
          size="sm"
          variant="ghost"
          aria-expanded={showComment}
          onClick={() => setShowComment((v) => !v)}
        >
          {showComment ? 'Hide options' : 'Comment…'}
        </Button>
      </div>
      {showComment && (
        <div className="space-y-2">
          <Textarea
            aria-label="Feedback comment"
            maxLength={MAX_COMMENT_LENGTH}
            placeholder="Optional comment (plain text)"
            value={comment}
            onChange={(e) => setComment(e.target.value)}
          />
          {maintainer && (
            <>
              <SuppressOption value={suppress} onChange={setSuppress} />
              <p className="text-xs text-muted-foreground">
                Suppression applies when you choose Intentional or Not relevant.
              </p>
            </>
          )}
        </div>
      )}
      {feedback.data?.mine && (
        <p className="text-xs text-muted-foreground" role="status">
          Your verdict: {VERDICTS.find((v) => v.value === feedback.data?.mine?.verdict)?.label}
          {feedback.data.mine.comment ? ` · “${feedback.data.mine.comment}”` : ''}
        </p>
      )}
    </div>
  );
}
