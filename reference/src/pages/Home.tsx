import styles from "./Home.module.css";

// protopie:begin home-flow
function HomeAbove() {
	return <></>;
}

function HomeBelow() {
	return <></>;
}
// protopie:end home-flow

export default function Home() {
	return (
		<>
			<HomeAbove />
			{/* protopie:begin hero_1 */}
			<main id="hero_1" data-protopie-id="hero_1" class={styles.hero}>
				<h1 class={styles.title}>Hello World</h1>
				<p class={styles.subtitle}>Built with SolidJS and TypeScript.</p>
			</main>
			{/* protopie:end hero_1 */}
			<HomeBelow />
		</>
	);
}
