import { useId } from 'react'

export function HeroBackground(props: React.ComponentPropsWithoutRef<'svg'>) {
  let id = useId()

  return (
    <svg
      aria-hidden="true"
      viewBox="0 0 668 1069"
      width={668}
      height={1069}
      fill="none"
      {...props}
    >
      <defs>
        <radialGradient
          id={`${id}-glow`}
          cx="0.5"
          cy="0.5"
          r="0.5"
          gradientUnits="objectBoundingBox"
        >
          <stop stopColor="#F59E0B" stopOpacity="0.3" />
          <stop offset="1" stopColor="#F59E0B" stopOpacity="0" />
        </radialGradient>
        <radialGradient
          id={`${id}-center-glow`}
          cx="334"
          cy="430"
          r="300"
          gradientUnits="userSpaceOnUse"
        >
          <stop stopColor="#F59E0B" stopOpacity="0.08" />
          <stop offset="0.6" stopColor="#F59E0B" stopOpacity="0.02" />
          <stop offset="1" stopColor="#F59E0B" stopOpacity="0" />
        </radialGradient>
      </defs>

      {/* Ambient center glow */}
      <rect
        width="668"
        height="1069"
        fill={`url(#${id}-center-glow)`}
      />

      <g opacity="0.35" strokeWidth={1.5}>
        {/* Concentric diamond shapes - precision targeting reticle */}
        <path
          d="M334 370 L394 430 L334 490 L274 430 Z"
          stroke="#44403C"
          opacity="0.6"
        />
        <path
          d="M334 310 L454 430 L334 550 L214 430 Z"
          stroke="#44403C"
          opacity="0.5"
        />
        <path
          d="M334 230 L534 430 L334 630 L134 430 Z"
          stroke="#44403C"
          opacity="0.4"
        />
        <path
          d="M334 130 L634 430 L334 730 L34 430 Z"
          stroke="#44403C"
          opacity="0.3"
        />
        <path
          d="M334 10 L754 430 L334 850 L-86 430 Z"
          stroke="#44403C"
          opacity="0.15"
        />

        {/* Radiating lines from center - laser beams */}
        {/* Cardinal directions */}
        <line x1="334" y1="430" x2="334" y2="0" stroke="#44403C" opacity="0.5" />
        <line x1="334" y1="430" x2="334" y2="1069" stroke="#44403C" opacity="0.5" />
        <line x1="334" y1="430" x2="0" y2="430" stroke="#44403C" opacity="0.3" />
        <line x1="334" y1="430" x2="668" y2="430" stroke="#44403C" opacity="0.3" />

        {/* Diagonal beams */}
        <line x1="334" y1="430" x2="0" y2="96" stroke="#44403C" opacity="0.25" />
        <line x1="334" y1="430" x2="668" y2="96" stroke="#44403C" opacity="0.25" />
        <line x1="334" y1="430" x2="0" y2="764" stroke="#44403C" opacity="0.25" />
        <line x1="334" y1="430" x2="668" y2="764" stroke="#44403C" opacity="0.25" />

        {/* Shallower angle beams */}
        <line x1="334" y1="430" x2="0" y2="280" stroke="#44403C" opacity="0.15" />
        <line x1="334" y1="430" x2="668" y2="280" stroke="#44403C" opacity="0.15" />
        <line x1="334" y1="430" x2="0" y2="580" stroke="#44403C" opacity="0.15" />
        <line x1="334" y1="430" x2="668" y2="580" stroke="#44403C" opacity="0.15" />

        {/* Steep angle beams */}
        <line x1="334" y1="430" x2="134" y2="0" stroke="#44403C" opacity="0.15" />
        <line x1="334" y1="430" x2="534" y2="0" stroke="#44403C" opacity="0.15" />
        <line x1="334" y1="430" x2="134" y2="1069" stroke="#44403C" opacity="0.15" />
        <line x1="334" y1="430" x2="534" y2="1069" stroke="#44403C" opacity="0.15" />
      </g>

      {/* Horizontal scan lines */}
      <g opacity="0.12" strokeWidth={1} stroke="#44403C">
        <line x1="0" y1="230" x2="668" y2="230" />
        <line x1="0" y1="330" x2="668" y2="330" />
        <line x1="0" y1="530" x2="668" y2="530" />
        <line x1="0" y1="630" x2="668" y2="630" />
        <line x1="0" y1="730" x2="668" y2="730" />
      </g>

      {/* Intersection nodes - amber accent on diamond vertices */}
      <g>
        {/* Inner diamond vertices */}
        <circle cx="334" cy="370" r="3" fill="#F59E0B" opacity="0.6" />
        <circle cx="394" cy="430" r="3" fill="#F59E0B" opacity="0.6" />
        <circle cx="334" cy="490" r="3" fill="#F59E0B" opacity="0.6" />
        <circle cx="274" cy="430" r="3" fill="#F59E0B" opacity="0.6" />

        {/* Second diamond vertices */}
        <circle cx="334" cy="310" r="4" fill="#F59E0B" opacity="0.45" />
        <circle cx="454" cy="430" r="4" fill="#F59E0B" opacity="0.45" />
        <circle cx="334" cy="550" r="4" fill="#F59E0B" opacity="0.45" />
        <circle cx="214" cy="430" r="4" fill="#F59E0B" opacity="0.45" />

        {/* Third diamond vertices */}
        <circle cx="334" cy="230" r="5" fill="#F59E0B" opacity="0.3" />
        <circle cx="534" cy="430" r="5" fill="#F59E0B" opacity="0.3" />
        <circle cx="334" cy="630" r="5" fill="#F59E0B" opacity="0.3" />
        <circle cx="134" cy="430" r="5" fill="#F59E0B" opacity="0.3" />

        {/* Outer diamond vertices */}
        <circle cx="334" cy="130" r="6" fill="#F59E0B" opacity="0.15" />
        <circle cx="634" cy="430" r="6" fill="#F59E0B" opacity="0.15" />
        <circle cx="334" cy="730" r="6" fill="#F59E0B" opacity="0.15" />
        <circle cx="34" cy="430" r="6" fill="#F59E0B" opacity="0.15" />

        {/* Center focal point - bright */}
        <circle cx="334" cy="430" r="8" fill="#F59E0B" opacity="0.5" />
        <circle cx="334" cy="430" r="3" fill="#FBBF24" opacity="0.8" />

        {/* Scattered accent nodes along beam intersections */}
        <circle cx="167" cy="264" r="3" fill="#F59E0B" opacity="0.2" />
        <circle cx="501" cy="264" r="3" fill="#F59E0B" opacity="0.2" />
        <circle cx="167" cy="596" r="3" fill="#F59E0B" opacity="0.2" />
        <circle cx="501" cy="596" r="3" fill="#F59E0B" opacity="0.2" />
        <circle cx="234" cy="330" r="2.5" fill="#F59E0B" opacity="0.25" />
        <circle cx="434" cy="330" r="2.5" fill="#F59E0B" opacity="0.25" />
        <circle cx="234" cy="530" r="2.5" fill="#F59E0B" opacity="0.25" />
        <circle cx="434" cy="530" r="2.5" fill="#F59E0B" opacity="0.25" />

        {/* Distant dim nodes */}
        <circle cx="100" cy="430" r="2" fill="#44403C" opacity="0.4" />
        <circle cx="568" cy="430" r="2" fill="#44403C" opacity="0.4" />
        <circle cx="334" cy="160" r="2" fill="#44403C" opacity="0.4" />
        <circle cx="334" cy="700" r="2" fill="#44403C" opacity="0.4" />
        <circle cx="334" cy="850" r="2" fill="#44403C" opacity="0.3" />
      </g>

      {/* Dashed targeting rings around center */}
      <circle
        cx="334"
        cy="430"
        r="40"
        stroke="#F59E0B"
        strokeWidth="1"
        opacity="0.15"
        strokeDasharray="4 6"
      />
      <circle
        cx="334"
        cy="430"
        r="80"
        stroke="#F59E0B"
        strokeWidth="0.75"
        opacity="0.1"
        strokeDasharray="6 10"
      />
    </svg>
  )
}
