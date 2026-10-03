import { Loading } from "solid-js";
import { Router } from "./router";

// protopie:begin layout-top
function LayoutTop() {
	return <></>;
}
// protopie:end layout-top

export default function App() {
	return (
		<Router>
			{(props) => (
				<>
					<LayoutTop />
					<Loading>{props.children}</Loading>
				</>
			)}
		</Router>
	);
}
