'use client';

import { zodResolver } from '@hookform/resolvers/zod';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { useForm } from 'react-hook-form';
import { Button } from '@/components/ui/button';
import { Label, Select, Textarea } from '@/components/ui/input';
import { errorMessage } from '@/components/ui/states';
import { ApiError } from '@/lib/api-client';
import { updateRepositorySettings } from '@/lib/api/endpoints';
import type { Repository } from '@/lib/api/pending';
import { keys } from '@/lib/queries';
import {
  REVIEWER_TYPES,
  repoSettingsFormSchema,
  toFormValues,
  toPatch,
  type RepoSettingsForm,
} from '@/lib/repo-settings';

function Toggle({ label, ...props }: { label: string } & React.ComponentProps<'input'>) {
  return (
    <label className="flex items-center gap-2 text-sm">
      <input type="checkbox" className="size-4" {...props} />
      {label}
    </label>
  );
}

/** Target branches, drafts, bots and reviewer toggles. Read-only for viewers. */
export function RepoSettingsForm({
  repository,
  maintainer,
}: {
  repository: Repository;
  maintainer: boolean;
}) {
  const queryClient = useQueryClient();
  const form = useForm<RepoSettingsForm>({
    resolver: zodResolver(repoSettingsFormSchema),
    defaultValues: toFormValues(repository.settings),
  });
  const save = useMutation({
    mutationFn: (values: RepoSettingsForm) =>
      updateRepositorySettings(repository.id, toPatch(values)),
    onSuccess: async (settings) => {
      form.reset(toFormValues(settings));
      await queryClient.invalidateQueries({ queryKey: keys.repository(repository.id) });
    },
  });
  const errors = form.formState.errors;
  const disabled = !maintainer || save.isPending;

  return (
    <form
      aria-label="Repository settings"
      className="space-y-4"
      onSubmit={form.handleSubmit((values) => save.mutate(values))}
      noValidate
    >
      <fieldset disabled={disabled} className="space-y-4">
        <div className="flex flex-wrap gap-6">
          <Toggle label="Enabled for review" {...form.register('enabled')} />
          <Toggle label="Skip draft pull requests" {...form.register('skip_drafts')} />
          <Toggle label="Skip bot authors" {...form.register('skip_bots')} />
        </div>

        <div className="space-y-1">
          <Label htmlFor="target_branches">Target branches</Label>
          <Textarea
            id="target_branches"
            placeholder="main, release/*"
            aria-invalid={errors.target_branches ? true : undefined}
            aria-describedby="target_branches_help"
            {...form.register('target_branches')}
          />
          <p id="target_branches_help" className="text-xs text-muted-foreground">
            Comma or newline separated. Empty reviews every base branch; a trailing * is a prefix
            match.
          </p>
          {errors.target_branches && (
            <p role="alert" className="text-xs text-destructive">
              {errors.target_branches.message}
            </p>
          )}
        </div>

        <div className="space-y-2">
          <p className="text-sm font-medium">Reviewers</p>
          <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">
            {REVIEWER_TYPES.map((reviewer) => (
              <label key={reviewer} className="flex items-center justify-between gap-2 text-sm">
                <span className="capitalize">{reviewer}</span>
                <Select
                  className="w-32"
                  aria-label={`${reviewer} reviewer`}
                  {...form.register(`reviewer_overrides.${reviewer}`)}
                >
                  <option value="default">Policy default</option>
                  <option value="on">On</option>
                  <option value="off">Off</option>
                </Select>
              </label>
            ))}
          </div>
        </div>
      </fieldset>

      {maintainer && (
        <div className="flex items-center gap-3">
          <Button type="submit" disabled={save.isPending || !form.formState.isDirty}>
            {save.isPending ? 'Saving…' : 'Save settings'}
          </Button>
          {save.isSuccess && !form.formState.isDirty && (
            <span role="status" className="text-sm text-muted-foreground">
              Saved.
            </span>
          )}
          {save.isError && (
            <span role="alert" className="text-sm text-destructive">
              {save.error instanceof ApiError && save.error.status === 403
                ? 'Requires maintainer role.'
                : errorMessage(save.error, 'Could not save settings.')}
            </span>
          )}
        </div>
      )}
    </form>
  );
}
