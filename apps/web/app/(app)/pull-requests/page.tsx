import type { Metadata } from 'next';
import { Suspense } from 'react';
import { PullsView } from '@/components/pulls/PullsView';
import { Loading } from '@/components/ui/states';

export const metadata: Metadata = { title: 'Pull Requests' };

export default function PullRequestsPage() {
  return (
    <Suspense fallback={<Loading label="Loading pull requests" />}>
      <PullsView />
    </Suspense>
  );
}
