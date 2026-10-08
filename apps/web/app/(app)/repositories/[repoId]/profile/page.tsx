import type { Metadata } from 'next';
import { ProfileView } from '@/components/profile/ProfileView';

export const metadata: Metadata = { title: 'Repository profile' };

export default async function ProfilePage({ params }: { params: Promise<{ repoId: string }> }) {
  const { repoId } = await params;
  return <ProfileView repoId={repoId} />;
}
