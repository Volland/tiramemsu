"""Illustrative correction policy: exclude old evidence, preserve historical content."""
import json,sys
from pathlib import Path
old={'root':{'employer-a'},'confidence':{'root'},'evidence':{'root'},'membership':{'root','session'},'membership-author':{'membership'}}
cascade=set(old);exclude={'confidence','evidence','membership'}
retained=cascade-exclude
while True:
 next_set={x for x in retained if (old[x]&cascade)<=retained}
 if next_set==retained:break
 retained=next_set
assert retained=={'root'}
snapshot={x:set(y) for x,y in old.items()}
new={'root-v2':{'employer-b'},'lineage':{'root-v2','root'}}
# Revalidation creates fresh evidence and confidence, rather than copying old annotations.
new.update({'confidence-v2':{'root-v2'},'evidence-v2':{'root-v2'}})
assert 'evidence' not in new and 'confidence' not in new
assert old==snapshot
allocated=set(old)|set(new)|{'session','employer-a','employer-b'}
assert all(refs<=allocated for refs in new.values())
assert all(refs<=set(new)|{'employer-b'} for key,refs in new.items() if key!='lineage')
result={'status':'PASS','scenario':'exclude object-sensitive annotations and foreign membership, close retention, patch employer, revalidate with fresh occurrences','retained':sorted(retained),'historical_content_preserved':old==snapshot,'limitation':'Application-policy model check, not a database integration test or factual evaluation.'}
print(json.dumps(result,indent=2))
if len(sys.argv)>1:Path(sys.argv[1]).write_text(json.dumps(result,indent=2)+'\n')
