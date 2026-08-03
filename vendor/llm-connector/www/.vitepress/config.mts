import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'llm-connector',
  description: 'High-performance Rust library for unifying LLM providers behind one type-safe API.',

  appearance: 'dark',

  head: [
    ['link', { rel: 'icon', href: '/favicon.png' }],
    ['link', { rel: 'preconnect', href: 'https://fonts.googleapis.com' }],
    ['link', { rel: 'preconnect', href: 'https://fonts.gstatic.com', crossorigin: '' }],
    [
      'link',
      {
        href: 'https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;700&display=swap',
        rel: 'stylesheet'
      }
    ],
    ['meta', { property: 'og:title', content: 'llm-connector' }],
    [
      'meta',
      {
        property: 'og:description',
        content: 'A Rust-first connector for OpenAI/Anthropic/Gemini and more — unified streaming, tools and multimodal.'
      }
    ],
    ['meta', { name: 'twitter:card', content: 'summary_large_image' }]
  ],

  themeConfig: {
    logo: '/logo.png',
    siteTitle: 'llm-connector',

    nav: [
      { text: 'Guide', link: '/guide/getting-started' },
      { text: 'Examples', link: '/examples/' },
      { text: 'Blog', link: '/blog/' },
      { text: 'Dev', link: '/dev/' },
      {
        text: 'v1.1.5',
        items: [
          { text: 'Changelog', link: 'https://github.com/lipish/llm-connector/blob/main/CHANGELOG.md' },
          { text: 'Contributing', link: '/guide/contributing' }
        ]
      }
    ],

    sidebar: {
      '/dev/': [
        {
          text: 'Development',
          items: [
            { text: 'Status', link: '/dev/' }
          ]
        }
      ],
      '/guide/': [
        {
          text: 'Introduction',
          items: [
            { text: 'Getting Started', link: '/guide/getting-started' },
            { text: 'Providers', link: '/guide/providers' },
            { text: 'Streaming', link: '/guide/streaming' },
            { text: 'Tools', link: '/guide/tools' },
            { text: 'Multi-modal', link: '/guide/multimodal' }
          ]
        },
        {
          text: 'Reference',
          items: [
            { text: 'Architecture', link: '/guide/architecture' },
            { text: 'Migration', link: '/guide/migration' },
            { text: 'Contributing', link: '/guide/contributing' }
          ]
        },
        {
          text: 'API Analysis',
          items: [
            { text: 'OpenAI Responses API Analysis & Plan', link: '/guide/openai-responses-api-analysis-and-plan' }
          ]
        }
      ],
      '/examples/': [
        {
          text: 'Examples',
          items: [
            { text: 'Overview', link: '/examples/' }
          ]
        }
      ],
      '/blog/': [
        {
          text: 'Blog',
          items: [
            { text: 'Overview', link: '/blog/' }
          ]
        }
      ]
    },

    socialLinks: [
      { icon: 'github', link: 'https://github.com/lipish/llm-connector' },
      { icon: 'twitter', link: 'https://x.com/lipiisme' }
    ],

    footer: {
      message: 'Released under the MIT License.',
      copyright: 'Copyright © 2026 llm-connector Contributors'
    },

    search: {
      provider: 'local'
    }
  }
})
