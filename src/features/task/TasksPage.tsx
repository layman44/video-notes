import { JobTable, type JobActionHandlers } from "../../components/JobTable";
import type { Job } from "../../types";

interface TasksPageProps {
  jobs: Job[];
  onOpenJob: (job: Job) => void;
  jobActions: JobActionHandlers;
}

export function TasksPage({ jobs, onOpenJob, jobActions }: TasksPageProps) {
  return (
    <section className="standard-page page-frame">
      <header className="page-header">
        <div>
          <h1>任务</h1>
          <p>查看、继续或重新整理已经处理的视频。</p>
        </div>
      </header>
      <JobTable jobs={jobs} onOpen={onOpenJob} {...jobActions} />
    </section>
  );
}
