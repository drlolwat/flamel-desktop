import SplashScreen from "./SplashScreen";
export default function LoginPage({
  onComplete,
}: { onComplete?: () => void; flamelRegistered?: boolean } = {}) {
  return <SplashScreen onConnect={() => onComplete?.()} />;
}
