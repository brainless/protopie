import { Loading } from "solid-js";
import type { ParentProps } from "solid-js";
import { Router } from "./router";

// protopie:begin providers
function Providers(props: ParentProps) {
	return <>{props.children}</>;
}
// protopie:end providers

// protopie:begin layout-top
function LayoutTop() {
	return <></>;
}
// protopie:end layout-top

export default function App() {
	return (
		<Router>
			{(props) => (
				<Providers>
					<LayoutTop />
					<Loading>{props.children}</Loading>
				</Providers>
			)}
		</Router>
	);
}
