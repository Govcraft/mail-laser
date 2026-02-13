function LogomarkPaths() {
  return (
    <g
      fill="none"
      stroke="#F59E0B"
      strokeLinejoin="round"
      strokeLinecap="round"
    >
      {/* Diamond shape - echoes hero background, suggests envelope fold */}
      <path d="M18 3 L33 18 L18 33 L3 18 Z" strokeWidth={2.5} />
      {/* Horizontal laser beam through center */}
      <line x1="1" y1="18" x2="35" y2="18" strokeWidth={2} />
      {/* Center focal point */}
      <circle cx="18" cy="18" r="2" fill="#F59E0B" stroke="none" />
    </g>
  )
}

export function Logomark(props: React.ComponentPropsWithoutRef<'svg'>) {
  return (
    <svg aria-hidden="true" viewBox="0 0 36 36" fill="none" {...props}>
      <LogomarkPaths />
    </svg>
  )
}

export function Logo(props: React.ComponentPropsWithoutRef<'svg'>) {
  return (
    <svg aria-hidden="true" viewBox="0 0 165 36" fill="none" {...props}>
      <LogomarkPaths />
      <text
        x="44"
        y="24.5"
        style={{
          fontSize: '20px',
          fontWeight: 700,
          fontFamily:
            'var(--font-bricolage-grotesque), system-ui, sans-serif',
          letterSpacing: '-0.03em',
        }}
      >
        MailLaser
      </text>
    </svg>
  )
}
