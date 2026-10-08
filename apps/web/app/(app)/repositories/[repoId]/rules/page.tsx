import type { Metadata } from 'next';
import { RulesView } from '@/components/rules/RulesView';

export const metadata: Metadata = { title: 'Rules' };

export default async function RepositoryRulesPage({
  params,
}: {
  params: Promise<{ repoId: string }>;
}) {
  const { repoId } = await params;
  return <RulesView repoId={repoId} />;
}
