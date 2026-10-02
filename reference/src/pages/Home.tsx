import styles from "./Home.module.css";

export default function Home() {
	return (
		<main class={styles.hero}>
			<h1 class={styles.title}>Hello World</h1>
			<p class={styles.subtitle}>Built with SolidJS and TypeScript.</p>
		</main>
	);
}
