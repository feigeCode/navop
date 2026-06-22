import Image from "next/image";
import Link from "next/link";
import { Bot, ChevronDown, Database, KeyRound, Server, Sparkles } from "lucide-react";
import { localePath, type Dictionary, type Locale } from "@/lib/i18n";

const icons = [Database, Server, Bot, KeyRound];
const metrics = [
  { emphasis: "14+", label: "engines" },
  { label: "SSH / SFTP" },
  { label: "RDP / VNC" },
  { label: "AI SQL" }
];

export function MarketingHome({ locale, dict }: { locale: Locale; dict: Dictionary }) {
  const features = [
    [dict.features.database, dict.features.databaseCopy],
    [dict.features.ssh, dict.features.sshCopy],
    [dict.features.ai, dict.features.aiCopy],
    [dict.features.team, dict.features.teamCopy]
  ];

  return (
    <main>
      <HeroSection dict={dict} />
      <CapabilitySection dict={dict} />
      <FeatureSection dict={dict} features={features} />
      <TeamSection dict={dict} />
      <CtaSection locale={locale} dict={dict} />
    </main>
  );
}

function HeroSection({ dict }: { dict: Dictionary }) {
  return (
    <section className="hero">
      <div className="hero-stage" aria-hidden="true" />
      <div className="container hero-layout">
        <div className="hero-content">
          <span className="eyebrow"><Sparkles size={15} /> {dict.home.eyebrow}</span>
          <h1>{dict.home.title}</h1>
          <p>
            {dict.home.subtitleLines.map((line) => (
              <span key={line}>{line}</span>
            ))}
          </p>
          <div className="hero-metrics">
            {metrics.map((metric) => (
              <span key={metric.label}>
                {metric.emphasis ? <strong>{metric.emphasis}</strong> : null}
                {metric.emphasis ? " " : null}
                {metric.label}
              </span>
            ))}
          </div>
        </div>
        <div className="hero-preview" aria-label="OnetCli product preview">
          <div className="hero-preview-frame">
            <Image className="hero-preview-image" src="/screenshots/app.png" alt="OnetCli workspace" width={2572} height={1670} priority />
          </div>
        </div>
        <a className="scroll-cue" href="#capabilities" aria-label="Scroll to product capabilities">
          <ChevronDown size={22} />
        </a>
      </div>
    </section>
  );
}

function CapabilitySection({ dict }: { dict: Dictionary }) {
  return (
    <section className="section section-capabilities" id="capabilities">
      <div className="container capability-overview">
        <div className="section-header">
          <h2 className="section-title">{dict.home.capabilitiesTitle}</h2>
          <p className="section-copy">{dict.home.capabilitiesCopy}</p>
        </div>
        <div className="capability-grid">
          {dict.home.capabilities.map((capability, index) => {
            const Icon = icons[index];
            return (
              <article className="capability-item" key={capability.title}>
                <div className="feature-icon"><Icon size={22} /></div>
                <h3>{capability.title}</h3>
                <p>{capability.copy}</p>
              </article>
            );
          })}
        </div>
      </div>
    </section>
  );
}

function FeatureSection({ dict, features }: { dict: Dictionary; features: string[][] }) {
  return (
    <section className="section section-quiet" id="features">
      <div className="container">
        <div className="section-header">
          <h2 className="section-title">{dict.home.featuresTitle}</h2>
          <p className="section-copy">{dict.home.featuresCopy}</p>
        </div>
        <div className="feature-grid">
          {features.map(([title, copy], index) => {
            const Icon = icons[index];
            return (
              <article className="feature-card" key={title}>
                <div className="feature-icon"><Icon size={24} /></div>
                <span className="feature-index">0{index + 1}</span>
                <h3>{title}</h3>
                <p>{copy}</p>
              </article>
            );
          })}
        </div>
      </div>
    </section>
  );
}

function TeamSection({ dict }: { dict: Dictionary }) {
  return (
    <section className="section section-showcase">
      <div className="container showcase">
        <div className="showcase-copy stack">
          <span className="eyebrow">{dict.home.teamEyebrow}</span>
          <h2 className="section-title">{dict.home.teamTitle}</h2>
          <p className="section-copy">{dict.home.teamCopy}</p>
        </div>
        <div className="showcase-image-frame">
          <Image className="product-shot" src="/screenshots/database.png" alt="OnetCli database workspace" width={2572} height={1670} />
        </div>
      </div>
    </section>
  );
}

function CtaSection({ locale, dict }: { locale: Locale; dict: Dictionary }) {
  return (
    <section className="section section-cta">
      <div className="container cta-band">
        <h2 className="section-title">{dict.home.ctaTitle}</h2>
        <p className="section-copy">{dict.home.ctaCopy}</p>
        <Link className="button primary" href={localePath(locale, "/register")}>
          {dict.auth.registerButton}
        </Link>
      </div>
    </section>
  );
}
