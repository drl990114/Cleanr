import Translate from '@docusaurus/Translate';
import Heading from '@theme/Heading';
import type {ReactNode} from 'react';
import styles from './styles.module.css';

const features = [
  {id: 'inspect', step: '01', title: <Translate id="home.featureInspect">Find the candidates</Translate>, description: <Translate id="home.featureInspectText">Inspect one project or known cache locations. Scanning and reviewing do not change your files.</Translate>},
  {id: 'review', step: '02', title: <Translate id="home.featureReview">Understand the tradeoff</Translate>, description: <Translate id="home.featureReviewText">See the size, matching reason, confidence, and rebuild risk. Recent or uncertain items are not automatically selected.</Translate>},
  {id: 'recover', step: '03', title: <Translate id="home.featureRecover">Confirm what moves</Translate>, description: <Translate id="home.featureRecoverText">Your selection is checked again before it moves to system trash. Local history records results and supports best-effort restore.</Translate>},
];

export default function HomepageFeatures(): ReactNode {
  return (
    <section className={styles.features}>
      <div className="container">
        <div className="row">
          {features.map(({id, step, title, description}) => (
            <div className="col col--4" key={id}>
              <p className={styles.step} aria-hidden="true">{step}</p>
              <Heading as="h2" className={styles.title}>{title}</Heading>
              <p>{description}</p>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
