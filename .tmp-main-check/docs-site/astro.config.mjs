import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import starlightOpenAPI, { openAPISidebarGroups } from "starlight-openapi";

// Interactive API reference (mika#175). The two YAML specs are
// auto-generated from the Rust source via utoipa and validated in CI;
// the plugin renders them as interactive Starlight pages.
const openAPIConfig = [
  {
    base: "api/mika-spirit",
    label: "mika-spirit",
    schema: "../docs/openapi/mika-spirit.yaml",
  },
  {
    base: "api/gateway",
    label: "mika-gateway",
    schema: "../docs/openapi/gateway.yaml",
  },
];

export default defineConfig({
  site: "https://mika-docs.senara-solutions.ai",
  integrations: [
    starlight({
      title: "Mika",
      description:
        "Documentation for Mika — a conversation-first AI executive assistant",
      logo: {
        src: "./src/assets/mika-logo.svg",
        replacesTitle: true,
      },
      social: [
        {
          icon: "github",
          label: "GitHub",
          href: "https://github.com/senara-solutions/mika",
        },
      ],
      editLink: {
        baseUrl: "https://github.com/senara-solutions/mika/edit/main/docs/",
      },
      lastUpdated: false,
      pagination: true,
      favicon: "/favicon.svg",
      tableOfContents: { minHeadingLevel: 2, maxHeadingLevel: 3 },
      customCss: ["./src/styles/custom.css"],
      plugins: [starlightOpenAPI(openAPIConfig)],
      sidebar: [
        { slug: "getting-started", label: "Getting Started" },
        { slug: "architecture", label: "Architecture" },
        { slug: "configuration", label: "Configuration" },
        { slug: "runtime-structure", label: "Runtime Structure" },
        { slug: "deployment", label: "Deployment" },
        { slug: "skills", label: "Skills" },
        { slug: "slash-commands", label: "Slash Commands" },
        {
          label: "API Reference",
          collapsed: false,
          items: openAPISidebarGroups,
        },
        {
          label: "Architecture Decisions",
          collapsed: true,
          autogenerate: { directory: "adr" },
        },
      ],
    }),
  ],
});
