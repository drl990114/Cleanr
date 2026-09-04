import Link from '@docusaurus/Link';
import Translate, {translate} from '@docusaurus/Translate';
import useBaseUrl from '@docusaurus/useBaseUrl';
import HomepageFeatures from '@site/src/components/HomepageFeatures';
import CodeBlock from '@theme/CodeBlock';
import Heading from '@theme/Heading';
import Layout from '@theme/Layout';
import type {ReactNode} from 'react';

import styles from './index.module.css';

export default function Home(): ReactNode {
  const preview = useBaseUrl('/img/cleanr-scan.png');
  const recording = useBaseUrl('/media/cleanr-first-scan.mp4');
  return (
    <Layout
      title={translate({id: 'home.title', message: 'Understand your developer caches'})}
      description={translate({id: 'home.description', message: 'Review developer caches in your terminal, understand each candidate, and move only confirmed items to system trash. Open source, with an optional agent workflow.'})}>
      <header className={styles.heroBanner}>
        <div className="container">
          <p className={styles.eyebrow}>Cleanr · <Translate id="home.eyebrow">Open source · Terminal first</Translate></p>
          <Heading as="h1" className={styles.title}>
            <Translate id="home.heading">Understand your caches. Review before you clean.</Translate>
          </Heading>
          <p className={styles.subtitle}>
            <Translate id="home.subtitle">Find generated files across old projects and developer tools. See why they matched, choose what goes, and keep a path back through system trash.</Translate>
          </p>
          <div className={styles.buttons}>
            <Link className="button button--primary button--lg" to="/docs/quick-start">
              <Translate id="home.start">Start with a read-only scan</Translate>
            </Link>
            <Link className="button button--outline button--primary button--lg" to="/docs/evidence-and-privacy">
              <Translate id="home.agent">Use with an agent</Translate>
            </Link>
          </div>
          <div className={styles.install}>
            <CodeBlock language="bash">npm install --global cleanr-cli</CodeBlock>
          </div>
          <p className={styles.platforms}>
            <Translate id="home.platforms">macOS · Linux · Windows</Translate>
            {' · '}<Link to="/docs/support-matrix"><Translate id="home.support">Platform support and limits</Translate></Link>
          </p>
          <figure className={styles.preview}>
            <img src={preview} width="1200" height="720" fetchPriority="high"
              alt={translate({id: 'home.previewAlt', message: 'Cleanr terminal showing a read-only scan of generated sample projects.'})} />
            <figcaption><Translate id="home.previewCaption">A real terminal session with generated sample projects. Candidate sizes are not a measurement of freed disk space.</Translate></figcaption>
          </figure>
        </div>
      </header>
      <main>
        <HomepageFeatures />
        <section className={styles.walkthrough} aria-labelledby="walkthrough-title">
          <div className="container">
            <Heading as="h2" id="walkthrough-title"><Translate id="home.walkthrough">See the first scan</Translate></Heading>
            <p><Translate id="home.walkthroughDescription">Scan a single project, inspect the candidates, then leave without changing files.</Translate></p>
            <video className={styles.video} controls preload="none" poster={preview} width="1200" height="720" aria-labelledby="walkthrough-title">
              <source src={recording} type="video/mp4" />
              <Translate id="home.videoFallback">Your browser does not support video playback.</Translate>
            </video>
            <p className={styles.caption}><Link to="/docs/demo"><Translate id="home.transcript">Read the walkthrough and reproduction steps</Translate></Link></p>
          </div>
        </section>
        <section className={styles.boundary}>
          <div className="container">
            <Heading as="h2"><Translate id="home.boundaryTitle">Know what happens next</Translate></Heading>
            <p><Translate id="home.trashBoundary">Confirmed items move to system trash. Space is not necessarily freed until you empty it yourself; after that, Cleanr cannot restore those items.</Translate>{' '}<Link to="/docs/safety-and-recovery"><Translate id="home.safetyLink">Recovery limits</Translate></Link></p>
            <p><Translate id="home.aiBoundary">Cleanr does not upload your analysis. If you use a hosted AI agent, the agent may send tool output to its provider. Decide what it may read before sharing a report.</Translate>{' '}<Link to="/docs/evidence-and-privacy"><Translate id="home.privacyLink">Agent data boundary</Translate></Link></p>
          </div>
        </section>
      </main>
    </Layout>
  );
}
