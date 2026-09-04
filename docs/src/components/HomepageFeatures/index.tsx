import Translate from '@docusaurus/Translate';
import Heading from '@theme/Heading';
import type {ReactNode} from 'react';
import styles from './styles.module.css';

const features = [
  {id: 'inspect', step: '01', title: <Translate id="home.featureInspect">Built for AI agents</Translate>, description: <Translate id="home.featureInspectText">Structured, read-only scan results give your agent sizes, matching reasons, confidence, and risk notes to work with.</Translate>},
  {id: 'review', step: '02', title: <Translate id="home.featureReview">Review before cleanup</Translate>, description: <Translate id="home.featureReviewText">Review reasons and risks, then choose what goes. Confirmation is on by default; agent cleanup requires your authorization for the exact plan.</Translate>},
  {id: 'recover', step: '03', title: <Translate id="home.featureRecover">Checks before every move</Translate>, description: <Translate id="home.featureRecoverText">Cleanr rechecks selected paths and file state, rejecting protected paths and symbolic-link targets. Items go to system Trash with local records for best-effort restore.</Translate>},
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
