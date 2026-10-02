import { redirect } from 'next/navigation';
import { getSession } from '@/lib/session';

/** Placeholder authenticated landing page; WEB-002 replaces it with the app shell + dashboard. */
export default async function HomePage() {
  const session = await getSession();
  if (!session) redirect('/login');

  return (
    <main className="p-8">
      <h1 className="text-2xl font-semibold">ReviewGraph</h1>
      <p className="mt-2 text-sm text-muted-foreground">
        Signed in as {session.user.name ?? session.user.login}.
      </p>
    </main>
  );
}
