import base from '@reviewgraph/config/eslint';
import nextPlugin from '@next/eslint-plugin-next';

export default [
  ...base,
  {
    plugins: { '@next/next': nextPlugin },
    rules: { ...nextPlugin.configs['core-web-vitals'].rules },
  },
  {
    rules: {
      // Untrusted text is rendered as text, never as HTML.
      'no-restricted-syntax': [
        'error',
        {
          selector: "JSXAttribute[name.name='dangerouslySetInnerHTML']",
          message: 'dangerouslySetInnerHTML is banned: render untrusted text as text.',
        },
      ],
    },
  },
  { ignores: ['next-env.d.ts', '.next/**'] },
];
