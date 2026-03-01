import { Nav } from "./components/Nav";
import { Hero } from "./components/Hero";
import { Features } from "./components/Features";
import { Teams } from "./components/Teams";
import { HowItWorks } from "./components/HowItWorks";
import { OpenSource } from "./components/OpenSource";
import { Footer } from "./components/Footer";

export default function App() {
  return (
    <div className="min-h-screen bg-bg font-sans text-muted">
      <Nav />
      <Hero />
      <Features />
      <Teams />
      <HowItWorks />
      <OpenSource />
      <Footer />
    </div>
  );
}
