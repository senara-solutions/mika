import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";

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
      sidebar: [
        { slug: "getting-started", label: "Getting Started" },
        { slug: "architecture", label: "Architecture" },
        { slug: "configuration", label: "Configuration" },
        { slug: "runtime-structure", label: "Runtime Structure" },
        { slug: "deployment", label: "Deployment" },
        { slug: "skills", label: "Skills" },
        { slug: "slash-commands", label: "Slash Commands" },
        {
          label: "Architecture Decisions",
          collapsed: true,
          autogenerate: { directory: "adr" },
        },
      ],
    }),
  ],
});
