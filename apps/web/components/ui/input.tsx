import * as React from 'react';
import { cn } from '@/lib/utils';

const fieldClass =
  'h-9 w-full min-w-0 rounded-md border bg-background px-3 py-1 text-sm shadow-xs outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-destructive';

function Input({ className, ...props }: React.ComponentProps<'input'>) {
  return <input data-slot="input" className={cn(fieldClass, className)} {...props} />;
}

function Select({ className, ...props }: React.ComponentProps<'select'>) {
  return <select data-slot="select" className={cn(fieldClass, 'px-2', className)} {...props} />;
}

function Textarea({ className, ...props }: React.ComponentProps<'textarea'>) {
  return (
    <textarea
      data-slot="textarea"
      className={cn(fieldClass, 'h-auto min-h-16 py-2', className)}
      {...props}
    />
  );
}

function Label({ className, ...props }: React.ComponentProps<'label'>) {
  return <label data-slot="label" className={cn('text-sm font-medium', className)} {...props} />;
}

export { Input, Label, Select, Textarea };
