import Link from 'next/link';
import { Button } from '@/components/ui/button';

export default function NotFound() {
  return (
    <main className="flex min-h-screen flex-col items-center justify-center gap-4 p-6 text-center">
      <h1 className="text-2xl font-semibold">Page not found</h1>
      <p className="text-sm text-muted-foreground">That page does not exist or was moved.</p>
      <Button asChild variant="outline">
        <Link href="/">Back to the dashboard</Link>
      </Button>
    </main>
  );
}
