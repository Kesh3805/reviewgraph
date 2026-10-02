import type { Metadata } from 'next';
import { redirect } from 'next/navigation';
import { Button } from '@/components/ui/button';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { getSession } from '@/lib/session';

export const metadata: Metadata = { title: 'Sign in' };

export default async function LoginPage({
  searchParams,
}: {
  searchParams: Promise<{ next?: string }>;
}) {
  if (await getSession()) redirect('/');

  const { next } = await searchParams;
  // Only same-origin relative paths are honoured as a post-login destination.
  const safeNext = next && next.startsWith('/') && !next.startsWith('//') ? next : undefined;
  const loginHref = `/api/v1/auth/github/login${safeNext ? `?next=${encodeURIComponent(safeNext)}` : ''}`;

  return (
    <main className="flex min-h-screen items-center justify-center p-6">
      <Card className="w-full max-w-sm">
        <CardHeader>
          <CardTitle className="text-xl">ReviewGraph</CardTitle>
          <CardDescription>
            Sign in to review pull requests with repository context.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <Button asChild className="w-full">
            <a href={loginHref}>Continue with GitHub</a>
          </Button>
        </CardContent>
      </Card>
    </main>
  );
}
