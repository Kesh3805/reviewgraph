import type { Metadata } from 'next';
import { IntelligenceView } from '@/components/intelligence/IntelligenceView';

export const metadata: Metadata = { title: 'Repository intelligence' };

export default async function IntelligencePage({
  params,
}: {
  params: Promise<{ repoId: string }>;
}) {
  const { repoId } = await params;
  return <IntelligenceView repoId={repoId} />;
}
