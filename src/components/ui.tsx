import type { ReactNode } from 'react';
export function PageHeading({
  eyebrow,
  title,
  description,
  action,
}: {
  eyebrow: string;
  title: string;
  description: string;
  action?: ReactNode;
}) {
  return (
    <header className="page-heading">
      <div>
        {eyebrow && <p className="eyebrow">{eyebrow}</p>}
        <h1>{title}</h1>
        <p className="muted">{description}</p>
      </div>
      {action}
    </header>
  );
}
export function Empty({ children }: { children: ReactNode }) {
  return <p className="empty">{children}</p>;
}
export function Stat({
  label,
  value,
  detail,
  detailTitle,
  truncateDetail,
}: {
  label: string;
  value: string;
  detail: string;
  detailTitle?: string;
  truncateDetail?: boolean;
}) {
  return (
    <article className="panel stat">
      <span className="muted">{label}</span>
      <strong>{value}</strong>
      <small
        title={detailTitle}
        className={truncateDetail ? 'truncate' : undefined}
      >
        {detail}
      </small>
    </article>
  );
}
